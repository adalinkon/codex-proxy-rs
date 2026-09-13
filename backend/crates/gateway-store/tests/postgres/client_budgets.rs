use std::time::SystemTime;

use chrono::{DateTime, Utc};
use gateway_admin::{
    model::{MutationActor, MutationContext, client_keys::UpdateClientKey},
    ports::store::ClientKeyStore as _,
};
use gateway_core::{
    engine::{
        ModelRequestId,
        budget::{ClientBudgetCharge, ClientBudgetPort, ClientBudgetStatus},
    },
    error::GatewayErrorKind,
    policy::{ClientApiKeyId, RateLimits},
};
use gateway_store::postgres::{
    ClientApiKeyRepository as _, PgAdminClientKeyStore, PgClientApiKeyRepository,
    PgClientBudgetStore,
};

use super::TestDatabase;

fn key_id(key: &str) -> ClientApiKeyId {
    ClientApiKeyId::new(key).unwrap()
}

fn charge(key: &str, request: &str, amount: &str) -> ClientBudgetCharge {
    ClientBudgetCharge {
        user_id: Some("test-owner".to_owned()),
        key_id: key_id(key),
        request_id: ModelRequestId::new(format!("req_{request}")).unwrap(),
        amount_usd: amount.parse().unwrap(),
        completed_at: SystemTime::now(),
    }
}

async fn seed(database: &TestDatabase, key: &str, daily: &str, weekly: &str) {
    sqlx::query(
        "insert into client_api_keys (user_id, id, name, key, daily_limit_usd, weekly_limit_usd, created_at, updated_at)
        values ('test-owner', $1, $1, $2, $3::text::numeric, $4::text::numeric, now(), now())",
    )
    .bind(key)
    .bind(format!("sk_{key:a<43}"))
    .bind(daily)
    .bind(weekly)
    .execute(&database.pool)
    .await
    .unwrap();
}

async fn status(database: &TestDatabase, key: &str) -> ClientBudgetStatus {
    PgClientApiKeyRepository::new(database.pool.clone())
        .get_client_api_key(key)
        .await
        .unwrap()
        .unwrap()
        .budget
}

fn context() -> MutationContext {
    MutationContext {
        actor: MutationActor::System,
        request_id: "budget-test".to_owned(),
    }
}

#[tokio::test]
async fn unchanged_windows_are_not_rewritten_by_admission_or_duplicate_settlement() {
    let Some(database) = TestDatabase::create("budgets_unchanged").await else {
        return;
    };
    seed(&database, "key", "1", "2").await;
    let store = PgClientBudgetStore::new(database.pool.clone());
    let (first, second) = tokio::join!(
        store.admit("test-owner".to_owned(), key_id("key")),
        store.admit("test-owner".to_owned(), key_id("key")),
    );
    first.unwrap();
    second.unwrap();
    let settled = charge("key", "unchanged", "0.25");
    store.settle(settled.clone()).await.unwrap();
    let versions = || {
        sqlx::query_scalar::<_, String>(
            "select xmin::text from user_budget_windows where user_id='test-owner'
         union all select xmin::text from client_key_budget_windows where client_api_key_id='key'",
        )
        .fetch_all(&database.pool)
    };
    let before = versions().await.unwrap();
    assert_eq!(before.len(), 2);
    for _ in 0..3 {
        store
            .admit("test-owner".to_owned(), key_id("key"))
            .await
            .unwrap();
    }
    store.settle(settled).await.unwrap();
    assert_eq!(
        versions().await.unwrap(),
        before,
        "unchanged windows must keep their row versions"
    );
    let budget = status(&database, "key").await;
    assert_eq!(budget.daily_used_usd.canonical(), "0.25");
    assert_eq!(budget.weekly_used_usd.canonical(), "0.25");
    database.close().await;
}

#[tokio::test]
async fn admission_uses_returned_rollover_amounts_and_commits_before_rejection() {
    let Some(database) = TestDatabase::create("budgets_returned_rollover").await else {
        return;
    };
    seed(&database, "key", "1", "3").await;
    sqlx::query(
        "update admin_users set daily_limit_usd=1,weekly_limit_usd=3 where id='test-owner'",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    let store = PgClientBudgetStore::new(database.pool.clone());
    store.settle(charge("key", "prior-day", "1")).await.unwrap();
    sqlx::raw_sql("update user_budget_windows set daily_start=daily_start-interval '1 day',daily_end=daily_end-interval '1 day';
        update client_key_budget_windows set daily_start=daily_start-interval '1 day',daily_end=daily_end-interval '1 day';")
        .execute(&database.pool).await.unwrap();
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    let amounts: Vec<(String, String)> = sqlx::query_as(
        "select daily_used_usd::text,weekly_used_usd::text from user_budget_windows
         union all select daily_used_usd::text,weekly_used_usd::text from client_key_budget_windows",
    ).fetch_all(&database.pool).await.unwrap();
    assert_eq!(
        amounts,
        vec![("0.0000000000".into(), "1.0000000000".into()); 2]
    );

    // 日窗口推进后仍可被周限额拒绝，但新日边界必须提交，不能重复推进或清空周用量。
    sqlx::raw_sql("update admin_users set weekly_limit_usd=1 where id='test-owner';
        update user_budget_windows set daily_start=daily_start-interval '1 day',daily_end=daily_end-interval '1 day',daily_used_usd=1;
        update client_key_budget_windows set daily_start=daily_start-interval '1 day',daily_end=daily_end-interval '1 day',daily_used_usd=1;")
        .execute(&database.pool).await.unwrap();
    assert_eq!(
        store
            .admit("test-owner".to_owned(), key_id("key"))
            .await
            .unwrap_err()
            .client_error_code(),
        Some("user_weekly_budget_exceeded")
    );
    let used: Vec<String> = sqlx::query_scalar(
        "select daily_used_usd::text from user_budget_windows union all select daily_used_usd::text from client_key_budget_windows",
    ).fetch_all(&database.pool).await.unwrap();
    assert_eq!(used, vec!["0.0000000000"; 2]);
    database.close().await;
}

#[tokio::test]
async fn admission_reads_amounts_committed_while_waiting_for_owner_lock() {
    let Some(database) = TestDatabase::create("budgets_waiting_admission").await else {
        return;
    };
    seed(&database, "key", "1", "3").await;
    sqlx::query("update admin_users set daily_limit_usd=1 where id='test-owner'")
        .execute(&database.pool)
        .await
        .unwrap();
    let store = PgClientBudgetStore::new(database.pool.clone());
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    let mut tx = database.pool.begin().await.unwrap();
    sqlx::query("select id from admin_users where id='test-owner' for no key update")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut admission =
        tokio::spawn(async move { store.admit("test-owner".to_owned(), key_id("key")).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut admission)
            .await
            .is_err()
    );
    sqlx::query("update user_budget_windows set daily_used_usd=1 where user_id='test-owner'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        admission.await.unwrap().unwrap_err().client_error_code(),
        Some("user_daily_budget_exceeded")
    );
    database.close().await;
}

#[tokio::test]
async fn budgets_settle_exactly_once_and_enforce_each_threshold_across_store_instances() {
    let Some(database) = TestDatabase::create("budgets_exact").await else {
        return;
    };
    seed(&database, "day", "0.3", "2").await;
    seed(&database, "week", "2", "0.2").await;
    let first = PgClientBudgetStore::new(database.pool.clone());
    let second = PgClientBudgetStore::new(database.pool.clone());
    for (key, prefix, amount) in [("day", "d", "0.1"), ("week", "w", "0.1")] {
        for _ in 0..3 {
            // 已准入请求可完成并超过限额，不预占估算费用。
            first
                .admit("test-owner".to_owned(), key_id(key))
                .await
                .unwrap();
        }
        for index in 0..3 {
            let id = format!("{prefix}-{index}");
            let (a, b) = tokio::join!(
                first.settle(charge(key, &id, amount)),
                second.settle(charge(key, &id, amount))
            );
            a.unwrap();
            b.unwrap();
        }
    }
    let day = status(&database, "day").await;
    assert_eq!(day.daily_used_usd.canonical(), "0.3");
    assert_eq!(day.weekly_used_usd.canonical(), "0.3");
    let day_error = second
        .admit("test-owner".to_owned(), key_id("day"))
        .await
        .unwrap_err();
    assert_eq!(day_error.kind(), GatewayErrorKind::RateLimited);
    assert_eq!(
        day_error.client_error_code(),
        Some("key_daily_budget_exceeded")
    );
    assert!(day_error.retry_after().is_some());
    let week_error = first
        .admit("test-owner".to_owned(), key_id("week"))
        .await
        .unwrap_err();
    assert_eq!(
        week_error.client_error_code(),
        Some("key_weekly_budget_exceeded")
    );
    let event_count: i64 = sqlx::query_scalar("select count(*) from client_key_charge_events")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(event_count, 6, "rejected admission must not create charges");
    let user_events: (i64, String) = sqlx::query_as(
        "select count(*),sum(amount_usd)::text from user_charge_events where user_id='test-owner'",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(user_events, (6, "0.6000000000".to_owned()));
    database.close().await;
}

#[tokio::test]
async fn window_rollover_is_shanghai_midnight_and_seven_days_with_late_settlement() {
    let Some(database) = TestDatabase::create("budgets_windows").await else {
        return;
    };
    seed(&database, "key", "1", "2").await;
    let store = PgClientBudgetStore::new(database.pool.clone());
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    let (day_start, day_end, week_end): (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as("select daily_start, daily_end, weekly_end from client_key_budget_windows where client_api_key_id = 'key'")
            .fetch_one(&database.pool).await.unwrap();
    assert_eq!(day_start.timestamp().rem_euclid(86400), 16 * 3600);
    assert_eq!((day_end - day_start).num_hours(), 24);
    assert_eq!((week_end - day_start).num_hours(), 168);
    store.settle(charge("key", "first", "1")).await.unwrap();
    sqlx::query("update client_key_budget_windows set daily_start = daily_start - interval '1 day', daily_end = daily_start")
        .execute(&database.pool).await.unwrap();
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    assert_eq!(
        status(&database, "key").await.daily_used_usd.canonical(),
        "0"
    );
    assert_eq!(
        status(&database, "key").await.weekly_used_usd.canonical(),
        "1"
    );
    store.settle(charge("key", "second", "1")).await.unwrap();
    sqlx::query("update client_key_budget_windows set daily_end = now() - interval '1 second', weekly_end = now() - interval '1 second'")
        .execute(&database.pool).await.unwrap();
    let virtual_reset = status(&database, "key").await;
    assert_eq!(virtual_reset.daily_used_usd.canonical(), "0");
    assert_eq!(virtual_reset.weekly_used_usd.canonical(), "0");
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    let old = ClientBudgetCharge {
        user_id: Some("test-owner".to_owned()),
        completed_at: (day_start - chrono::Duration::seconds(1)).into(),
        ..charge("key", "after-reset", "0.9")
    };
    store.settle(old).await.unwrap();
    let reset = status(&database, "key").await;
    assert_eq!(reset.daily_used_usd.canonical(), "0");
    assert_eq!(reset.weekly_used_usd.canonical(), "0");
    assert!(reset.weekly_resets_at.is_some());
    database.close().await;
}

#[tokio::test]
async fn zero_cost_and_interrupted_requests_never_block_limited_keys() {
    let Some(database) = TestDatabase::create("budgets_zero").await else {
        return;
    };
    seed(&database, "key", "1", "5").await;
    let store = PgClientBudgetStore::new(database.pool.clone());
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    store.settle(charge("key", "no-cost", "0")).await.unwrap();
    // 模拟准入后进程退出，没有费用可结算；重启后仍应允许同一 Key 使用。
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    let restarted = PgClientBudgetStore::new(database.pool.clone());
    restarted
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    let events: i64 = sqlx::query_scalar("select count(*) from client_key_charge_events")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(events, 1, "admission must not leave pending charges");
    let user_events: i64 = sqlx::query_scalar("select count(*) from user_charge_events")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        user_events, 1,
        "zero-cost settlements retain both ledger events"
    );
    assert_eq!(
        status(&database, "key").await.daily_used_usd.canonical(),
        "0"
    );
    restarted
        .settle(charge("key", "known", "0.4"))
        .await
        .unwrap();
    assert_eq!(
        status(&database, "key").await.daily_used_usd.canonical(),
        "0.4"
    );
    database.close().await;
}

#[tokio::test]
async fn transient_settlement_failure_rolls_back_and_retries_exact_cost_before_admission() {
    let Some(database) = TestDatabase::create("budgets_retry").await else {
        return;
    };
    seed(&database, "key", "1", "5").await;
    let store = PgClientBudgetStore::new(database.pool.clone());
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    // 在费用事件插入后让窗口写入失败，验证整个事务回滚。
    sqlx::raw_sql(
        "create function reject_budget_update() returns trigger language plpgsql as $$
        begin
            if new.daily_used_usd > 0 then raise exception 'temporary test failure'; end if;
            return new;
        end $$;
        create trigger reject_budget_update before update on client_key_budget_windows
        for each row execute function reject_budget_update();",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    assert!(store.settle(charge("key", "retry", "1.25")).await.is_err());
    let user_totals = || {
        sqlx::query_as::<_, (i64, String)>(
            "select (select count(*) from user_charge_events),daily_used_usd::text
             from user_budget_windows where user_id='test-owner'",
        )
        .fetch_one(&database.pool)
    };
    assert_eq!(user_totals().await.unwrap(), (0, "0.0000000000".to_owned()));
    let events: i64 = sqlx::query_scalar("select count(*) from client_key_charge_events")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);
    assert_eq!(
        status(&database, "key").await.daily_used_usd.canonical(),
        "0"
    );
    sqlx::query("drop trigger reject_budget_update on client_key_budget_windows")
        .execute(&database.pool)
        .await
        .unwrap();
    let error = store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap_err();
    assert_eq!(error.client_error_code(), Some("key_daily_budget_exceeded"));
    store.settle(charge("key", "retry", "1.25")).await.unwrap();
    assert_eq!(user_totals().await.unwrap(), (1, "1.2500000000".to_owned()));
    assert_eq!(
        status(&database, "key").await.daily_used_usd.canonical(),
        "1.25"
    );
    database.close().await;
}

#[tokio::test]
async fn budget_updates_preserve_omitted_limits_and_do_not_clear_usage() {
    let Some(database) = TestDatabase::create("budgets_policy").await else {
        return;
    };
    seed(&database, "key", "0", "0").await;
    let store = PgClientBudgetStore::new(database.pool.clone());
    let admin = PgAdminClientKeyStore::new(database.pool.clone());
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    store
        .settle(charge("key", "unlimited", "2.75"))
        .await
        .unwrap();
    let update = UpdateClientKey {
        id: ClientApiKeyId::new("key").unwrap(),
        name: "key".to_owned(),
        label: None,
        group_ids: vec![],
        limits: RateLimits {
            max_concurrency: 3,
            requests_per_minute: 0,
        },
        daily_limit_usd: Some("2".parse().unwrap()),
        weekly_limit_usd: Some("10".parse().unwrap()),
    };
    admin
        .update_client_key(update.clone(), &context())
        .await
        .unwrap();
    admin
        .update_client_key(
            UpdateClientKey {
                daily_limit_usd: None,
                weekly_limit_usd: None,
                ..update.clone()
            },
            &context(),
        )
        .await
        .unwrap();
    let current = status(&database, "key").await;
    assert_eq!(current.limits.daily_usd.canonical(), "2");
    assert_eq!(current.limits.weekly_usd.canonical(), "10");
    assert_eq!(current.daily_used_usd.canonical(), "2.75");
    assert_eq!(
        store
            .admit("test-owner".to_owned(), key_id("key"))
            .await
            .unwrap_err()
            .client_error_code(),
        Some("key_daily_budget_exceeded")
    );
    admin
        .update_client_key(
            UpdateClientKey {
                daily_limit_usd: Some("0".parse().unwrap()),
                weekly_limit_usd: None,
                ..update
            },
            &context(),
        )
        .await
        .unwrap();
    store
        .admit("test-owner".to_owned(), key_id("key"))
        .await
        .unwrap();
    sqlx::query("update client_api_keys set enabled = false where id = 'key'")
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        store
            .admit("test-owner".to_owned(), key_id("key"))
            .await
            .unwrap_err()
            .kind(),
        GatewayErrorKind::PolicyDenied
    );
    sqlx::query("delete from client_api_keys where id = 'key'")
        .execute(&database.pool)
        .await
        .unwrap();
    assert_eq!(
        store
            .admit("test-owner".to_owned(), key_id("key"))
            .await
            .unwrap_err()
            .kind(),
        GatewayErrorKind::Unauthorized
    );
    store.settle(charge("key", "allowed", "1")).await.unwrap();
    database.close().await;
}

#[tokio::test]
async fn budget_database_outage_fails_closed() {
    let Some(database) = TestDatabase::create("budgets_outage").await else {
        return;
    };
    let store = PgClientBudgetStore::new(database.pool.clone());
    database.pool.close().await;
    assert_eq!(
        store
            .admit("test-owner".to_owned(), key_id("key"))
            .await
            .unwrap_err()
            .client_error_code(),
        Some("key_budget_unavailable")
    );
    assert!(store.settle(charge("key", "offline", "1")).await.is_err());
    database.close().await;
}
