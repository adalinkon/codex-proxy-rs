//! 按用户、Key 顺序串行检查限额，并幂等累计已取得的 USD 费用。

use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use gateway_core::{
    engine::budget::{
        ClientBudgetCharge, ClientBudgetError, ClientBudgetLimits, ClientBudgetPort,
        ClientBudgetStatus,
    },
    error::{GatewayError, GatewayErrorKind},
    metering::Decimal,
    policy::ClientApiKeyId,
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::{StoreResult, postgres_unavailable};

pub struct PgClientBudgetStore {
    pool: PgPool,
    retry: Mutex<BTreeMap<String, ClientBudgetCharge>>,
}

impl PgClientBudgetStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            retry: Mutex::new(BTreeMap::new()),
        }
    }

    async fn admit_inner(
        &self,
        user_id: String,
        key_id: ClientApiKeyId,
    ) -> Result<(), GatewayError> {
        // 短暂存储故障后按原金额重试；进程退出丢失的费用不转成人工核账或阻断 Key。
        let retries = self
            .retry
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .filter(|charge| charge.user_id.as_deref() == Some(user_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for charge in retries {
            self.settle(charge).await.map_err(|_| unavailable())?;
        }
        let mut tx = self.pool.begin().await.map_err(|_| unavailable())?;
        let user = sqlx::query("select enabled,daily_limit_usd::text,weekly_limit_usd::text from admin_users where id=$1 for no key update")
            .bind(&user_id).fetch_optional(&mut *tx).await.map_err(|_| unavailable())?
            .filter(|row| row.get::<bool,_>("enabled"))
            .ok_or_else(|| GatewayError::new(GatewayErrorKind::Unauthorized,"user is disabled or missing"))?;
        let row = sqlx::query(
            "select daily_limit_usd::text, weekly_limit_usd::text, enabled
            from client_api_keys where id = $1 and user_id=$2 for no key update",
        )
        .bind(key_id.as_str())
        .bind(&user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(|| {
            GatewayError::new(
                GatewayErrorKind::Unauthorized,
                "client API key no longer exists",
            )
        })?;
        if !row.get::<bool, _>("enabled") {
            return Err(GatewayError::new(
                GatewayErrorKind::PolicyDenied,
                "client API key is disabled",
            ));
        }
        let limits = |row: &sqlx::postgres::PgRow| -> Result<ClientBudgetLimits, GatewayError> {
            Ok(ClientBudgetLimits {
                daily_usd: row
                    .get::<String, _>("daily_limit_usd")
                    .parse()
                    .map_err(|_| unavailable())?,
                weekly_usd: row
                    .get::<String, _>("weekly_limit_usd")
                    .parse()
                    .map_err(|_| unavailable())?,
            })
        };
        let key_limits = limits(&row)?;
        let now = Utc::now();
        let user_window = advance_user_windows(&mut tx, &user_id, now)
            .await
            .map_err(|_| unavailable())?;
        let key_window = advance_windows(&mut tx, key_id.as_str(), now)
            .await
            .map_err(|_| unavailable())?;
        if let Some(error) = budget_rejection(
            limits(&user)?,
            &user_window,
            now,
            ["user_daily_budget_exceeded", "user_weekly_budget_exceeded"],
            "user budget is exhausted",
        )? {
            tx.commit().await.map_err(|_| unavailable())?;
            return Err(error);
        }
        if key_limits.is_limited()
            && let Some(error) = budget_rejection(
                key_limits,
                &key_window,
                now,
                ["key_daily_budget_exceeded", "key_weekly_budget_exceeded"],
                "client API key budget is exhausted",
            )?
        {
            tx.commit().await.map_err(|_| unavailable())?;
            return Err(error);
        }
        tx.commit().await.map_err(|_| unavailable())
    }

    async fn settle_inner(&self, charge: &ClientBudgetCharge) -> Result<(), ClientBudgetError> {
        let mut tx = self.pool.begin().await.map_err(|_| ClientBudgetError)?;
        let user_id = charge
            .user_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or(ClientBudgetError)?;
        // 准入和结算均先锁用户再锁 Key，跨 Key 结算与删除不会丢失用户费用。
        sqlx::query("select id from admin_users where id=$1 for no key update")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| ClientBudgetError)?;
        let key = sqlx::query_scalar::<_, String>(
            "select id from client_api_keys where id = $1 and user_id=$2 for no key update",
        )
        .bind(charge.key_id.as_str())
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| ClientBudgetError)?;
        advance_user_windows(&mut tx, user_id, Utc::now())
            .await
            .map_err(|_| ClientBudgetError)?;
        // RETURNING 只提供新事件，重复请求不参与累计；清零截止仍按原完成时间判断。
        sqlx::query(
            "with inserted as (
                insert into user_charge_events(request_id,user_id,client_api_key_ref,amount_usd,completed_at)
                values($1,$2,$3,$4::text::numeric,$5)
                on conflict(request_id) do nothing
                returning user_id,amount_usd,completed_at
            )
            update user_budget_windows w set
                daily_used_usd=w.daily_used_usd+case when c.completed_at>=w.daily_start and c.completed_at<w.daily_end then c.amount_usd else 0 end,
                weekly_used_usd=w.weekly_used_usd+case when c.completed_at>=w.weekly_start and c.completed_at<w.weekly_end then c.amount_usd else 0 end
            from inserted c join admin_users u on u.id=c.user_id
            where w.user_id=c.user_id and (u.budget_reset_at is null or c.completed_at>=u.budget_reset_at)",
        )
        .bind(charge.request_id.as_str())
        .bind(user_id)
        .bind(charge.key_id.as_str())
        .bind(charge.amount_usd.canonical())
        .bind(DateTime::<Utc>::from(charge.completed_at))
        .execute(&mut *tx)
        .await
        .map_err(|_| ClientBudgetError)?;
        if let Some(key) = key {
            settle_in_transaction(&mut tx, &key, charge)
                .await
                .map_err(|_| ClientBudgetError)?;
        }
        tx.commit().await.map_err(|_| ClientBudgetError)
    }
}

fn budget_rejection(
    limits: ClientBudgetLimits,
    window: &sqlx::postgres::PgRow,
    now: DateTime<Utc>,
    codes: [&'static str; 2],
    message: &'static str,
) -> Result<Option<GatewayError>, GatewayError> {
    for (limit, used_field, end_field, code) in [
        (limits.daily_usd, "daily_used_usd", "daily_end", codes[0]),
        (limits.weekly_usd, "weekly_used_usd", "weekly_end", codes[1]),
    ] {
        let used: Decimal = window
            .get::<String, _>(used_field)
            .parse()
            .map_err(|_| unavailable())?;
        if limit != Decimal::ZERO && used >= limit {
            let reset: DateTime<Utc> = window.get(end_field);
            return Ok(Some(
                GatewayError::new(GatewayErrorKind::RateLimited, message)
                    .with_client_code(code)
                    .with_retry_after((reset - now).to_std().unwrap_or(Duration::from_secs(1))),
            ));
        }
    }
    Ok(None)
}

async fn settle_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    charge: &ClientBudgetCharge,
) -> Result<(), sqlx::Error> {
    advance_windows(tx, key, Utc::now()).await?;
    // 与用户账本一致，仅累计本次实际插入的费用事件。
    sqlx::query(
        "with inserted as (
            insert into client_key_charge_events (request_id, client_api_key_id, amount_usd, completed_at)
            values ($1, $2, $3::text::numeric, $4)
            on conflict (request_id) do nothing
            returning client_api_key_id,amount_usd,completed_at
        )
        update client_key_budget_windows w set
            daily_used_usd=w.daily_used_usd+case when c.completed_at>=w.daily_start and c.completed_at<w.daily_end then c.amount_usd else 0 end,
            weekly_used_usd=w.weekly_used_usd+case when c.completed_at>=w.weekly_start and c.completed_at<w.weekly_end then c.amount_usd else 0 end
        from inserted c where w.client_api_key_id=c.client_api_key_id",
    )
    .bind(charge.request_id.as_str())
    .bind(key)
    .bind(charge.amount_usd.canonical())
    .bind(DateTime::<Utc>::from(charge.completed_at))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl ClientBudgetPort for PgClientBudgetStore {
    fn admit(
        &self,
        user_id: String,
        key_id: ClientApiKeyId,
    ) -> BoxFuture<'_, Result<(), GatewayError>> {
        Box::pin(async move { self.admit_inner(user_id, key_id).await })
    }

    fn settle(&self, charge: ClientBudgetCharge) -> BoxFuture<'_, Result<(), ClientBudgetError>> {
        Box::pin(async move {
            let result = self.settle_inner(&charge).await;
            let mut retry = self.retry.lock().map_err(|_| ClientBudgetError)?;
            if result.is_err() {
                retry.insert(charge.request_id.as_str().to_owned(), charge);
            } else {
                retry.remove(charge.request_id.as_str());
            }
            result
        })
    }
}

async fn advance_windows(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    now: DateTime<Utc>,
) -> Result<sqlx::postgres::PgRow, sqlx::Error> {
    advance_budget_windows(tx, key, now, false).await
}

async fn advance_user_windows(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    now: DateTime<Utc>,
) -> Result<sqlx::postgres::PgRow, sqlx::Error> {
    advance_budget_windows(tx, id, now, true).await
}

async fn advance_budget_windows(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    now: DateTime<Utc>,
    user: bool,
) -> Result<sqlx::postgres::PgRow, sqlx::Error> {
    let (table, column, owner, anchor) = if user {
        (
            "user_budget_windows",
            "user_id",
            "admin_users",
            "coalesce(o.budget_reset_at,o.created_at)",
        )
    } else {
        (
            "client_key_budget_windows",
            "client_api_key_id",
            "client_api_keys",
            "o.created_at",
        )
    };
    // 调用前必须已锁定所属用户；处理 Key 窗口时还须锁定 Key，保证快照包含此前提交的结算或重置。
    // CTE 的写入结果通过 RETURNING 返回；未变窗口直接读取，避免无效更新和额外往返。
    // 表名与列名仅来自上方固定分支，所有业务值仍使用绑定参数。
    sqlx::query(sqlx::AssertSqlSafe(format!("with advanced as (
        insert into {table}
        ({column}, daily_start, daily_end, weekly_start, weekly_end)
        select $1,p.daily_start,p.daily_end,p.weekly_start,p.weekly_end
        from {owner} o cross join lateral budget_periods({anchor},$2) p where o.id=$1
        and not exists (
            select 1 from {table} w where w.{column}=$1
            and (w.daily_start,w.daily_end,w.weekly_start,w.weekly_end)
                = (p.daily_start,p.daily_end,p.weekly_start,p.weekly_end)
        )
        on conflict ({column}) do update set
            daily_start = excluded.daily_start,
            daily_end = excluded.daily_end,
            daily_used_usd = case when {table}.daily_start=excluded.daily_start and {table}.daily_end=excluded.daily_end then {table}.daily_used_usd else 0 end,
            weekly_start = excluded.weekly_start,
            weekly_end = excluded.weekly_end,
            weekly_used_usd = case when {table}.weekly_start=excluded.weekly_start and {table}.weekly_end=excluded.weekly_end then {table}.weekly_used_usd else 0 end
        returning daily_used_usd::text,weekly_used_usd::text,daily_end,weekly_end
    )
    select * from advanced
    union all
    select daily_used_usd::text,weekly_used_usd::text,daily_end,weekly_end
    from {table} where {column}=$1 and not exists (select 1 from advanced)")))
        .bind(id).bind(now).fetch_one(&mut **tx).await
}

pub(super) async fn load_client_key_budgets(
    pool: &PgPool,
    records: &mut [super::ClientApiKeyRecord],
) -> StoreResult<()> {
    if records.is_empty() {
        return Ok(());
    }
    let ids = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        "select k.id, k.daily_limit_usd::text, k.weekly_limit_usd::text,
        w.daily_used_usd::text as daily_used,
        w.weekly_used_usd::text as weekly_used,
        w.daily_end, w.weekly_end
        from client_api_keys k join client_key_budget_status w on w.client_api_key_id = k.id
        where k.id = any($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(|_| postgres_unavailable("load client budgets"))?;
    let mut budgets = BTreeMap::new();
    for row in rows {
        let parse = |field| -> StoreResult<Decimal> {
            row.get::<String, _>(field)
                .parse()
                .map_err(|_| postgres_unavailable("decode client budget"))
        };
        budgets.insert(
            row.get::<String, _>("id"),
            ClientBudgetStatus {
                limits: ClientBudgetLimits {
                    daily_usd: parse("daily_limit_usd")?,
                    weekly_usd: parse("weekly_limit_usd")?,
                },
                daily_used_usd: parse("daily_used")?,
                weekly_used_usd: parse("weekly_used")?,
                daily_resets_at: row
                    .get::<Option<DateTime<Utc>>, _>("daily_end")
                    .map(Into::into),
                weekly_resets_at: row
                    .get::<Option<DateTime<Utc>>, _>("weekly_end")
                    .map(Into::into),
            },
        );
    }
    for record in records {
        record.budget = budgets
            .remove(&record.id)
            .ok_or_else(|| postgres_unavailable("load client budget policy"))?;
    }
    Ok(())
}

fn unavailable() -> GatewayError {
    GatewayError::new(
        GatewayErrorKind::ProviderInfrastructureUnavailable,
        "client budget service is temporarily unavailable",
    )
    .with_client_code("key_budget_unavailable")
}
