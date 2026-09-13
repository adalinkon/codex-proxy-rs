use super::TestDatabase;
use chrono::{Duration, Timelike as _, Utc};
use gateway_admin::{
    model::{
        client_keys::{DeleteClientKey, NewClientKey, OwnedKeyMutation},
        users::{UserPolicyUpdate, UserRole},
    },
    ports::store::ClientKeyStore as _,
};
use gateway_core::{
    engine::{
        ModelRequestId,
        budget::{ClientBudgetCharge, ClientBudgetPort},
    },
    policy::ClientApiKeyId,
};
use gateway_store::postgres::{
    PgAdminClientKeyStore, PgClientBudgetStore, PgRuntimeSnapshotRepository, PgUserRepository,
    RuntimeSnapshotRepository as _,
};
use std::time::SystemTime;

fn policy(id: &str, daily: &str, weekly: &str) -> UserPolicyUpdate {
    UserPolicyUpdate {
        id: id.to_owned(),
        role: UserRole::User,
        enabled: true,
        all_groups: false,
        group_ids: Vec::new(),
        limits: Default::default(),
        budget: gateway_core::engine::budget::ClientBudgetLimits {
            daily_usd: daily.parse().unwrap(),
            weekly_usd: weekly.parse().unwrap(),
        },
    }
}

#[tokio::test]
async fn owned_key_identity_updates_preserve_policy_and_check_owner() {
    let Some(db) = TestDatabase::create("owned_key_identity").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    for user in ["alice", "bob"] {
        users
            .save(policy(user, "5", "10"), Some("test-hash"))
            .await
            .unwrap();
    }
    create_key(&db, "alice", "identity-key").await;
    sqlx::query("update client_api_keys set daily_limit_usd=2.1234567891,weekly_limit_usd=8,max_concurrency=3,requests_per_minute=12 where id='identity-key'")
        .execute(&db.pool).await.unwrap();
    let mutation = OwnedKeyMutation::UpdateIdentity {
        id: ClientApiKeyId::new("identity-key").unwrap(),
        name: "renamed".into(),
        label: Some("personal".into()),
    };
    let keys = PgAdminClientKeyStore::new(db.pool.clone());
    assert!(
        keys.mutate_owned_key("bob", mutation.clone())
            .await
            .is_err()
    );
    keys.mutate_owned_key("alice", mutation.clone())
        .await
        .unwrap();
    let key = keys
        .reveal_client_key(&ClientApiKeyId::new("identity-key").unwrap())
        .await
        .unwrap()
        .unwrap()
        .record;
    assert_eq!(key.user_id, "alice");
    assert_eq!(key.name, "renamed");
    assert_eq!(key.label.as_deref(), Some("personal"));
    assert_eq!(key.budget.limits.daily_usd.canonical(), "2.1234567891");
    assert_eq!(key.budget.limits.weekly_usd.canonical(), "8");
    assert_eq!(key.limits.max_concurrency, 3);
    assert_eq!(key.limits.requests_per_minute, 12);
    let mut disabled = policy("alice", "5", "10");
    disabled.enabled = false;
    users.save(disabled, None).await.unwrap();
    assert!(keys.mutate_owned_key("alice", mutation).await.is_err());
    db.close().await;
}

#[tokio::test]
async fn deleting_user_revokes_keys_preserves_costs_and_prevents_resurrection() {
    let Some(db) = TestDatabase::create("user_deletion").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    users
        .save(policy("alice", "5", "10"), Some("test-hash"))
        .await
        .unwrap();
    create_key(&db, "alice", "delete-a").await;
    create_key(&db, "alice", "delete-b").await;
    let budgets = PgClientBudgetStore::new(db.pool.clone());
    admit(&budgets, "alice", "delete-a").await.unwrap();
    let charged = charge("alice", "delete-a", "before-delete", "0.5");
    budgets.settle(charged.clone()).await.unwrap();
    let revision = users.delete("alice").await.unwrap();
    assert!(revision.get() > 1);
    assert!(users.load("alice").await.unwrap().is_none());
    assert!(users.load_identity("alice").await.unwrap().is_none());
    assert!(
        users
            .list()
            .await
            .unwrap()
            .iter()
            .all(|user| user.identity.id != "alice")
    );
    let keys: i64 =
        sqlx::query_scalar("select count(*) from client_api_keys where user_id='alice'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(keys, 0);
    assert!(admit(&budgets, "alice", "delete-b").await.is_err());
    assert!(users.save(policy("alice", "0", "0"), None).await.is_err());
    assert!(
        users
            .save(policy("alice", "0", "0"), Some("new-hash"))
            .await
            .is_err()
    );
    assert!(
        !users
            .change_password("alice", None, "new-hash")
            .await
            .unwrap()
    );
    budgets.settle(charged).await.unwrap();
    let late = charge("alice", "delete-b", "after-delete", "0.1234567891");
    budgets.settle(late.clone()).await.unwrap();
    budgets.settle(late).await.unwrap();
    let (amount, count): (String, i64) = sqlx::query_as(
        "select sum(amount_usd)::text,count(*) from user_charge_events where user_id='alice'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(amount, "0.6234567891");
    assert_eq!(count, 2);
    let state: (bool, i64, bool) = sqlx::query_as(
        "select enabled,auth_revision,deleted_at is not null from admin_users where id='alice'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, (false, 1, true));
    assert!(users.delete("test-owner").await.is_err());
    assert!(
        users
            .load("test-owner")
            .await
            .unwrap()
            .unwrap()
            .identity
            .enabled
    );
    db.close().await;
}
async fn create_key(db: &TestDatabase, user: &str, id: &str) {
    PgAdminClientKeyStore::new(db.pool.clone())
        .mutate_owned_key(
            user,
            OwnedKeyMutation::Create(NewClientKey {
                user_id: Some(user.to_owned()),
                id: ClientApiKeyId::new(id).unwrap(),
                name: id.to_owned(),
                label: None,
                group_ids: Vec::new(),
                limits: Default::default(),
                budget: Default::default(),
                plaintext: format!("sk_{id:a<43}"),
            }),
        )
        .await
        .unwrap();
}
fn charge(user: &str, key: &str, id: &str, amount: &str) -> ClientBudgetCharge {
    ClientBudgetCharge {
        user_id: Some(user.to_owned()),
        key_id: ClientApiKeyId::new(key).unwrap(),
        request_id: ModelRequestId::new(format!("req_{id}")).unwrap(),
        amount_usd: amount.parse().unwrap(),
        completed_at: SystemTime::now(),
    }
}
async fn admit(
    store: &PgClientBudgetStore,
    user: &str,
    key: &str,
) -> Result<(), gateway_core::error::GatewayError> {
    store
        .admit(user.to_owned(), ClientApiKeyId::new(key).unwrap())
        .await
}

#[tokio::test]
async fn all_keys_share_user_money_and_deleting_keys_preserves_settlement() {
    let Some(db) = TestDatabase::create("user_shared_budgets").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    users
        .save(policy("alice", "1", "3"), Some("test-hash"))
        .await
        .unwrap();
    users
        .save(policy("bob", "1", "3"), Some("test-hash"))
        .await
        .unwrap();
    for (user, key) in [("alice", "a"), ("alice", "b"), ("bob", "c")] {
        create_key(&db, user, key).await;
    }
    let store = PgClientBudgetStore::new(db.pool.clone());
    admit(&store, "alice", "a").await.unwrap();
    admit(&store, "alice", "b").await.unwrap();
    assert!(admit(&store, "bob", "a").await.is_err());
    let a = charge("alice", "a", "request-a", "0.6");
    let b = charge("alice", "b", "request-b", "0.6");
    let (one, two) = tokio::join!(store.settle(a.clone()), store.settle(b));
    one.unwrap();
    two.unwrap();
    store.settle(a).await.unwrap();
    assert_eq!(
        users
            .load("alice")
            .await
            .unwrap()
            .unwrap()
            .budget
            .daily_used_usd
            .canonical(),
        "1.2"
    );
    assert_eq!(
        users
            .load("bob")
            .await
            .unwrap()
            .unwrap()
            .budget
            .daily_used_usd
            .canonical(),
        "0"
    );
    sqlx::query("update client_api_keys set daily_limit_usd=0.1,weekly_limit_usd=0.1 where id='b'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        admit(&store, "alice", "b")
            .await
            .unwrap_err()
            .client_error_code(),
        Some("user_daily_budget_exceeded")
    );
    users.save(policy("alice", "0", "1"), None).await.unwrap();
    assert_eq!(
        admit(&store, "alice", "b")
            .await
            .unwrap_err()
            .client_error_code(),
        Some("user_weekly_budget_exceeded")
    );
    users.save(policy("alice", "0", "0"), None).await.unwrap();
    assert_eq!(
        admit(&store, "alice", "b")
            .await
            .unwrap_err()
            .client_error_code(),
        Some("key_daily_budget_exceeded")
    );
    sqlx::query("update client_api_keys set daily_limit_usd=0 where id='b'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        admit(&store, "alice", "b")
            .await
            .unwrap_err()
            .client_error_code(),
        Some("key_weekly_budget_exceeded")
    );
    let keys = PgAdminClientKeyStore::new(db.pool.clone());
    assert!(
        keys.mutate_owned_key(
            "bob",
            OwnedKeyMutation::Delete(DeleteClientKey {
                id: ClientApiKeyId::new("a").unwrap()
            })
        )
        .await
        .is_err()
    );
    keys.mutate_owned_key(
        "alice",
        OwnedKeyMutation::Delete(DeleteClientKey {
            id: ClientApiKeyId::new("a").unwrap(),
        }),
    )
    .await
    .unwrap();
    let late = charge("alice", "a", "request-late", "0.1234567891");
    store.settle(late.clone()).await.unwrap();
    store.settle(late).await.unwrap();
    create_key(&db, "alice", "d").await;
    let alice = users.load("alice").await.unwrap().unwrap();
    assert_eq!(alice.budget.daily_used_usd.canonical(), "1.3234567891");
    assert_eq!(alice.key_count, 2);
    let user_event_count: i64 =
        sqlx::query_scalar("select count(*) from user_charge_events where user_id='alice'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(user_event_count, 3);
    let audit_count: i64 = sqlx::query_scalar("select count(*) from admin_audit_events")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);
    let mut disabled = policy("alice", "0", "0");
    disabled.enabled = false;
    users.save(disabled, None).await.unwrap();
    assert!(admit(&store, "alice", "d").await.is_err());
    let snapshot = PgRuntimeSnapshotRepository::new(db.pool.clone())
        .load_runtime_snapshot()
        .await
        .unwrap();
    assert!(
        !snapshot
            .client_api_keys
            .iter()
            .find(|key| key.id.as_str() == "d")
            .unwrap()
            .user
            .enabled
    );
    db.close().await;
}

#[tokio::test]
async fn windows_follow_beijing_days_and_retry_pending_costs_across_keys() {
    let Some(db) = TestDatabase::create("user_window_retry").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    users
        .save(policy("alice", "1", "4"), Some("test-hash"))
        .await
        .unwrap();
    create_key(&db, "alice", "a").await;
    create_key(&db, "alice", "b").await;
    let store = PgClientBudgetStore::new(db.pool.clone());
    admit(&store, "alice", "a").await.unwrap();
    let (day, week, end): (
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
    ) = sqlx::query_as(
        "select daily_start,weekly_start,weekly_end from user_budget_windows where user_id='alice'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((day + Duration::hours(8)).hour(), 0);
    assert_eq!(week, day);
    assert_eq!(end - week, Duration::days(7));
    store
        .settle(charge("alice", "a", "prior", "0.4"))
        .await
        .unwrap();
    sqlx::query("update user_budget_windows set daily_start=daily_start-interval '1 day',daily_end=daily_end-interval '1 day'").execute(&db.pool).await.unwrap();
    admit(&store, "alice", "b").await.unwrap();
    let status = users.load("alice").await.unwrap().unwrap().budget;
    assert_eq!(status.daily_used_usd.canonical(), "0");
    assert_eq!(status.weekly_used_usd.canonical(), "0.4");
    sqlx::query("update user_budget_windows set weekly_start=weekly_start-interval '7 days',weekly_end=weekly_end-interval '7 days'").execute(&db.pool).await.unwrap();
    admit(&store, "alice", "b").await.unwrap();
    assert_eq!(
        users
            .load("alice")
            .await
            .unwrap()
            .unwrap()
            .budget
            .weekly_used_usd
            .canonical(),
        "0"
    );
    let mut previous = charge("alice", "a", "previous-day", "0.3");
    previous.completed_at = (day - Duration::seconds(1)).into();
    store.settle(previous).await.unwrap();
    assert_eq!(
        users
            .load("alice")
            .await
            .unwrap()
            .unwrap()
            .budget
            .daily_used_usd
            .canonical(),
        "0"
    );
    sqlx::query("alter table user_charge_events rename to unavailable_user_charge_events")
        .execute(&db.pool)
        .await
        .unwrap();
    let pending = charge("alice", "a", "pending-cost", "1.1");
    assert!(store.settle(pending).await.is_err());
    sqlx::query("alter table unavailable_user_charge_events rename to user_charge_events")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        admit(&store, "alice", "b")
            .await
            .unwrap_err()
            .client_error_code(),
        Some("user_daily_budget_exceeded")
    );
    assert_eq!(
        users
            .load("alice")
            .await
            .unwrap()
            .unwrap()
            .budget
            .daily_used_usd
            .canonical(),
        "1.1"
    );
    db.close().await;
}

#[tokio::test]
async fn password_reset_revokes_generations_and_last_administrator_cannot_be_disabled() {
    let Some(db) = TestDatabase::create("user_password").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    users
        .save(policy("alice", "0", "0"), Some("initial-hash"))
        .await
        .unwrap();
    assert!(
        !users
            .change_password("alice", Some("wrong-hash"), "replacement")
            .await
            .unwrap()
    );
    assert_eq!(
        users
            .load_identity("alice")
            .await
            .unwrap()
            .unwrap()
            .auth_revision,
        0
    );
    assert!(
        users
            .change_password("alice", Some("initial-hash"), "replacement")
            .await
            .unwrap()
    );
    assert_eq!(
        users
            .load_identity("alice")
            .await
            .unwrap()
            .unwrap()
            .auth_revision,
        1
    );
    let mut disabled = policy("test-owner", "0", "0");
    disabled.role = UserRole::Admin;
    disabled.enabled = false;
    disabled.all_groups = true;
    assert!(users.save(disabled, None).await.is_err());
    assert!(
        users
            .load("test-owner")
            .await
            .unwrap()
            .unwrap()
            .identity
            .enabled
    );
    db.close().await;
}

#[tokio::test]
async fn migration_assigns_legacy_keys_and_preserves_password_limits_and_recorded_costs() {
    let Some(db) = TestDatabase::create_at("user_upgrade", 5).await else {
        return;
    };
    sqlx::raw_sql("insert into client_api_keys(id,name,key,max_concurrency,requests_per_minute,daily_limit_usd,weekly_limit_usd,created_at,updated_at)
        values('old-key','Legacy','sk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',2,30,5,20,now(),now());
        insert into client_key_budget_windows(client_api_key_id,daily_start,daily_end,weekly_start,weekly_end,daily_used_usd,weekly_used_usd)
        select 'old-key',day,day+interval '1 day',day,day+interval '7 days',1.1234567891,1.1234567891 from (select date_trunc('day',now() at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as day) d;
        insert into client_key_charge_events(request_id,client_api_key_id,amount_usd,completed_at) values('legacy-request','old-key',1.1234567891,now());")
        .execute(&db.pool).await.unwrap();
    super::TEST_MIGRATOR.run(&db.pool).await.unwrap();
    let user = PgUserRepository::new(db.pool.clone())
        .load("test-owner")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.identity.role, UserRole::Admin);
    assert!(user.all_groups);
    assert!(!user.budget.limits.is_limited());
    assert_eq!(user.budget.daily_used_usd.canonical(), "1.1234567891");
    let old:(String,String,String,i64,i64,String)=sqlx::query_as("select k.user_id,k.daily_limit_usd::text,w.daily_used_usd::text,k.max_concurrency,k.requests_per_minute,u.password_hash from client_api_keys k join client_key_budget_windows w on w.client_api_key_id=k.id join admin_users u on u.id=k.user_id where k.id='old-key'").fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        old,
        (
            "test-owner".to_owned(),
            "5.0000000000".to_owned(),
            "1.1234567891".to_owned(),
            2,
            30,
            "test-only-hash".to_owned()
        )
    );
    let no_owner=sqlx::query("insert into client_api_keys(id,name,key,created_at,updated_at) values('orphan','orphan','sk_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',now(),now())").execute(&db.pool).await;
    assert!(no_owner.is_err());
    db.close().await;
}

#[tokio::test]
async fn fixed_budget_periods_follow_creation_without_any_requests() {
    let Some(db) = TestDatabase::create("fixed_budget_periods").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    users
        .save(policy("alice", "0", "0"), Some("test-hash"))
        .await
        .unwrap();
    create_key(&db, "alice", "fixed-key").await;
    let day: chrono::DateTime<Utc> = sqlx::query_scalar(
        "select date_trunc('day',now() at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query("update admin_users set created_at=$1 where id='alice'")
        .bind(day - Duration::days(31))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("update client_api_keys set created_at=$1 where id='fixed-key'")
        .bind(day - Duration::days(9))
        .execute(&db.pool)
        .await
        .unwrap();
    let read = users.load("alice").await.unwrap().unwrap();
    assert_eq!(
        read.budget.daily_resets_at,
        Some((day + Duration::days(1)).into())
    );
    assert_eq!(
        read.budget.weekly_resets_at,
        Some((day + Duration::days(4)).into())
    );
    let keys = PgAdminClientKeyStore::new(db.pool.clone());
    let key = keys
        .reveal_client_key(&ClientApiKeyId::new("fixed-key").unwrap())
        .await
        .unwrap()
        .unwrap()
        .record;
    assert_eq!(
        key.budget.weekly_resets_at,
        Some((day + Duration::days(5)).into())
    );
    let count: i64 =
        sqlx::query_scalar("select count(*) from user_budget_windows where user_id='alice'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "只读查询不需要创建窗口或扫描账本");
    let store = PgClientBudgetStore::new(db.pool.clone());
    admit(&store, "alice", "fixed-key").await.unwrap();
    store
        .settle(charge("alice", "fixed-key", "fixed-charge", "0.25"))
        .await
        .unwrap();
    let mut disabled = policy("alice", "1", "2");
    disabled.enabled = false;
    users.save(disabled, None).await.unwrap();
    let next = users.load("alice").await.unwrap().unwrap();
    assert_eq!(next.budget.weekly_resets_at, read.budget.weekly_resets_at);
    assert_eq!(next.budget.weekly_used_usd.canonical(), "0.25");
    // 用固定时间验证整周边界、闰日和跨过多个空闲周期，不依赖测试执行时刻。
    for (at, expected) in [
        ("2024-02-29T00:00:00+08:00", "2024-03-07T00:00:00+08:00"),
        ("2024-03-06T23:59:59+08:00", "2024-03-07T00:00:00+08:00"),
        ("2024-03-07T00:00:00+08:00", "2024-03-14T00:00:00+08:00"),
        ("2024-04-01T12:00:00+08:00", "2024-04-04T00:00:00+08:00"),
    ] {
        let actual: chrono::DateTime<Utc> = sqlx::query_scalar("select weekly_end from budget_periods('2024-02-29T15:30:00+08:00',$1::text::timestamptz)").bind(at).fetch_one(&db.pool).await.unwrap();
        assert_eq!(
            actual,
            chrono::DateTime::parse_from_rfc3339(expected).unwrap()
        );
    }
    db.close().await;
}

#[tokio::test]
async fn budget_reset_is_idempotent_and_preserves_keys_history_and_late_charges() {
    let Some(db) = TestDatabase::create("user_budget_reset").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    for user in ["alice", "bob"] {
        users
            .save(policy(user, "1", "3"), Some("hash"))
            .await
            .unwrap();
    }
    for key in ["a", "b"] {
        create_key(&db, "alice", key).await;
    }
    let store = PgClientBudgetStore::new(db.pool.clone());
    let old = charge("alice", "a", "before-reset", "0.4");
    store.settle(old.clone()).await.unwrap();
    store
        .settle(charge("alice", "b", "before-reset-b", "0.5"))
        .await
        .unwrap();
    sqlx::query("update client_api_keys set daily_limit_usd=0.4 where id='a'")
        .execute(&db.pool)
        .await
        .unwrap();
    let operation = uuid::Uuid::now_v7().to_string();
    let (first, repeat) = tokio::join!(
        users.reset_budget("alice", &operation),
        users.reset_budget("alice", &operation)
    );
    let reset_at = first.unwrap();
    assert_eq!(repeat.unwrap(), reset_at);
    let user = users.load("alice").await.unwrap().unwrap();
    assert_eq!(user.budget.daily_used_usd.canonical(), "0");
    assert_eq!(user.budget.weekly_used_usd.canonical(), "0");
    assert_eq!(user.budget.limits.daily_usd.canonical(), "1");
    assert_eq!(user.identity.auth_revision, 0);
    assert_eq!(user.key_count, 2);
    assert_eq!(
        user.budget
            .weekly_resets_at
            .unwrap()
            .duration_since(user.budget.daily_resets_at.unwrap())
            .unwrap()
            .as_secs(),
        6 * 86400
    );
    assert_eq!(
        admit(&store, "alice", "a")
            .await
            .unwrap_err()
            .client_error_code(),
        Some("key_daily_budget_exceeded")
    );
    admit(&store, "alice", "b").await.unwrap();
    store
        .settle(charge("alice", "b", "after-reset", "0.2"))
        .await
        .unwrap();
    store.settle(old).await.unwrap();
    let mut late = charge("alice", "a", "late-before-reset", "0.3");
    late.completed_at = (reset_at - Duration::seconds(1)).into();
    store.settle(late.clone()).await.unwrap();
    store.settle(late).await.unwrap();
    assert_eq!(
        users.reset_budget("alice", &operation).await.unwrap(),
        reset_at
    );
    assert_eq!(
        users
            .load("alice")
            .await
            .unwrap()
            .unwrap()
            .budget
            .weekly_used_usd
            .canonical(),
        "0.2"
    );
    assert!(users.reset_budget("bob", &operation).await.is_err());
    assert!(
        users
            .reset_budget("missing", &uuid::Uuid::now_v7().to_string())
            .await
            .is_err()
    );
    let total: String = sqlx::query_scalar(
        "select sum(amount_usd)::text from user_charge_events where user_id='alice'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(total, "1.4000000000");
    let second = uuid::Uuid::now_v7().to_string();
    users.reset_budget("alice", &second).await.unwrap();
    store
        .settle(charge("alice", "b", "after-second-reset", "0.1"))
        .await
        .unwrap();
    users.reset_budget("alice", &operation).await.unwrap();
    assert_eq!(
        users
            .load("alice")
            .await
            .unwrap()
            .unwrap()
            .budget
            .daily_used_usd
            .canonical(),
        "0.1"
    );
    let audit: i64 = sqlx::query_scalar("select count(*) from admin_audit_events")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(audit, 0);
    db.close().await;
}

#[tokio::test]
async fn reset_serializes_with_settlement_and_keeps_the_cutoff_after_lock_acquisition() {
    let Some(db) = TestDatabase::create("budget_reset_race").await else {
        return;
    };
    let users = PgUserRepository::new(db.pool.clone());
    users
        .save(policy("alice", "1", "5"), Some("hash"))
        .await
        .unwrap();
    create_key(&db, "alice", "key").await;
    let store = PgClientBudgetStore::new(db.pool.clone());
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("select id from admin_users where id='alice' for no key update")
        .execute(&mut *tx)
        .await
        .unwrap();
    let completed = charge("alice", "key", "race", "0.5");
    let reset_users = users.clone();
    let reset = tokio::spawn(async move {
        reset_users
            .reset_budget("alice", &uuid::Uuid::now_v7().to_string())
            .await
    });
    let settle = tokio::spawn(async move { store.settle(completed).await });
    tx.commit().await.unwrap();
    reset.await.unwrap().unwrap();
    settle.await.unwrap().unwrap();
    let user = users.load("alice").await.unwrap().unwrap();
    assert_eq!(user.budget.daily_used_usd.canonical(), "0");
    let total: String = sqlx::query_scalar(
        "select sum(amount_usd)::text from user_charge_events where user_id='alice'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(total, "0.5000000000");
    db.close().await;
}

#[tokio::test]
async fn continuous_budget_migration_rebuilds_new_periods_and_refuses_missing_ledger() {
    let Some(db) = TestDatabase::create_at("budget_period_upgrade", 7).await else {
        return;
    };
    // 旧窗口从六天前起算，新窗口从三天前起算；五天前的费用保留在账本但不计入新周期。
    sqlx::raw_sql("update admin_users set created_at=now()-interval '31 days' where id='test-owner';
      insert into client_api_keys(user_id,id,name,key,created_at,updated_at) values('test-owner','old','old','sk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',now()-interval '31 days',now());
      insert into client_key_budget_windows(client_api_key_id,daily_start,daily_end,weekly_start,weekly_end,daily_used_usd,weekly_used_usd)
      select 'old',day,day+interval '24 hours',day-interval '6 days',day+interval '24 hours',0.2,0.7 from (select date_trunc('day',now() at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as day) d;
      insert into client_key_charge_events(request_id,client_api_key_id,amount_usd,completed_at) values('recent','old',0.2,now()),('earlier','old',0.5,now()-interval '5 days');")
      .execute(&db.pool).await.unwrap();
    // 先证明缺账会阻止升级，且失败事务不会留下部分迁移。
    sqlx::query(
        "update client_key_budget_windows set weekly_used_usd=0.8 where client_api_key_id='old'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut failed_connection = db.pool.acquire().await.unwrap();
    assert!(
        super::TEST_MIGRATOR
            .run(&mut *failed_connection)
            .await
            .is_err()
    );
    // 失败迁移仍持有会话级 advisory lock；关闭测试连接释放锁，再模拟修复后的重新启动。
    failed_connection.close().await.unwrap();
    sqlx::query(
        "update client_key_budget_windows set weekly_used_usd=0.7 where client_api_key_id='old'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::raw_sql("insert into user_charge_events(request_id,user_id,client_api_key_ref,amount_usd,completed_at) select request_id,'test-owner',client_api_key_id,amount_usd,completed_at from client_key_charge_events")
      .execute(&db.pool).await.unwrap();
    super::TEST_MIGRATOR.run(&db.pool).await.unwrap();
    let user = PgUserRepository::new(db.pool.clone())
        .load("test-owner")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.budget.weekly_used_usd.canonical(), "0.2");
    let key = PgAdminClientKeyStore::new(db.pool.clone())
        .reveal_client_key(&ClientApiKeyId::new("old").unwrap())
        .await
        .unwrap()
        .unwrap()
        .record;
    assert_eq!(key.budget.weekly_used_usd.canonical(), "0.2");
    let count: i64 = sqlx::query_scalar("select count(*) from user_charge_events")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    db.close().await;
}
