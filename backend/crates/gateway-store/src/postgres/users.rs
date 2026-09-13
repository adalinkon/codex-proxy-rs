//! 用户身份、授权和额度查询的 PostgreSQL 实现。

use chrono::{DateTime, Utc};
use gateway_admin::{
    model::{
        Revision,
        account_groups::{AccountGroupColor, AccountGroupRef},
        users::{UserIdentity, UserPolicyUpdate, UserRecord, UserRole},
    },
    ports::store::{AdminStoreError, AdminStoreErrorKind, AdminStoreResult},
};
use gateway_core::{
    engine::budget::{ClientBudgetLimits, ClientBudgetStatus},
    policy::RateLimits,
    routing::AccountGroupId,
};
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn load(&self, id: &str) -> AdminStoreResult<Option<UserRecord>> {
        Ok(self.query(Some(id)).await?.into_iter().next())
    }

    pub async fn load_identity(&self, id: &str) -> AdminStoreResult<Option<UserIdentity>> {
        sqlx::query("select id,role,enabled,auth_revision from admin_users where id=$1 and deleted_at is null")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(unavailable)?
            .as_ref()
            .map(user_identity)
            .transpose()
    }

    pub async fn list(&self) -> AdminStoreResult<Vec<UserRecord>> {
        self.query(None).await
    }

    pub async fn reset_budget(
        &self,
        id: &str,
        operation_id: &str,
    ) -> AdminStoreResult<DateTime<Utc>> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        // 与结算共用用户行锁；操作时间必须在取得锁后读取，避免把等待期间的费用算进新用量。
        sqlx::query_scalar::<_, String>(
            "select id from admin_users where id=$1 and deleted_at is null for no key update",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(unavailable)?
        .ok_or_else(|| {
            AdminStoreError::new(AdminStoreErrorKind::NotFound, "user", "user not found")
        })?;
        let created = sqlx::query_scalar::<_, DateTime<Utc>>(
            "insert into user_budget_reset_operations(operation_id,user_id,reset_at) values($1::text::uuid,$2,clock_timestamp()) on conflict(operation_id) do nothing returning reset_at"
        ).bind(operation_id).bind(id).fetch_optional(&mut *tx).await.map_err(write_error)?;
        let Some(reset_at) = created else {
            let prior: (String, DateTime<Utc>) = sqlx::query_as("select user_id,reset_at from user_budget_reset_operations where operation_id=$1::text::uuid")
                .bind(operation_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
            if prior.0 != id {
                return Err(AdminStoreError::new(
                    AdminStoreErrorKind::Conflict,
                    "user budget",
                    "reset operation belongs to another user",
                ));
            }
            tx.commit().await.map_err(unavailable)?;
            return Ok(prior.1);
        };
        sqlx::query("update admin_users set budget_reset_at=$2 where id=$1")
            .bind(id)
            .bind(reset_at)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("insert into user_budget_windows(user_id,daily_start,daily_end,weekly_start,weekly_end,daily_used_usd,weekly_used_usd)
            select $1,daily_start,daily_end,weekly_start,weekly_end,0,0 from budget_periods($2,$2)
            on conflict(user_id) do update set daily_start=excluded.daily_start,daily_end=excluded.daily_end,
            weekly_start=excluded.weekly_start,weekly_end=excluded.weekly_end,daily_used_usd=0,weekly_used_usd=0")
            .bind(id).bind(reset_at).execute(&mut *tx).await.map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        Ok(reset_at)
    }

    async fn query(&self, id: Option<&str>) -> AdminStoreResult<Vec<UserRecord>> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("set transaction isolation level repeatable read read only")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let rows = sqlx::query("select u.id,u.role,u.enabled,u.auth_revision,u.all_groups,u.max_concurrency,u.requests_per_minute,
            (select count(*) from client_api_keys k where k.user_id=u.id) as key_count,
            w.daily_used_usd::text as daily_used,
            w.weekly_used_usd::text as weekly_used,
            u.daily_limit_usd::text as daily_limit, u.weekly_limit_usd::text as weekly_limit,
            w.daily_end as daily_reset,
            w.weekly_end as weekly_reset
            from admin_users u join user_budget_status w on w.user_id=u.id
            where u.deleted_at is null and ($1::text is null or u.id=$1) order by u.created_at, u.id")
            .bind(id).fetch_all(&mut *tx).await.map_err(unavailable)?;
        let groups = sqlx::query("select ug.user_id, g.id, g.name, g.color, g.enabled from user_account_groups ug join account_groups g on g.id=ug.account_group_id where ($1::text is null or ug.user_id=$1) order by g.id")
            .bind(id).fetch_all(&mut *tx).await.map_err(unavailable)?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            let decimal = |field| {
                row.get::<String, _>(field)
                    .parse()
                    .map_err(|_| invalid("invalid user amount"))
            };
            let number = |field| {
                u64::try_from(row.get::<i64, _>(field)).map_err(|_| invalid("invalid user limit"))
            };
            records.push(UserRecord {
                identity: user_identity(&row)?,
                all_groups: row.get("all_groups"),
                groups: groups
                    .iter()
                    .filter(|g| g.get::<&str, _>("user_id") == id)
                    .map(|g| {
                        Ok(AccountGroupRef {
                            id: AccountGroupId::new(g.get::<String, _>("id"))
                                .map_err(|_| invalid("invalid group"))?,
                            name: g.get("name"),
                            enabled: g.get("enabled"),
                            color: AccountGroupColor::parse(g.get("color"))
                                .ok_or_else(|| invalid("invalid group color"))?,
                        })
                    })
                    .collect::<AdminStoreResult<_>>()?,
                limits: RateLimits {
                    max_concurrency: number("max_concurrency")?,
                    requests_per_minute: number("requests_per_minute")?,
                },
                budget: ClientBudgetStatus {
                    limits: ClientBudgetLimits {
                        daily_usd: decimal("daily_limit")?,
                        weekly_usd: decimal("weekly_limit")?,
                    },
                    daily_used_usd: decimal("daily_used")?,
                    weekly_used_usd: decimal("weekly_used")?,
                    daily_resets_at: row
                        .get::<Option<DateTime<Utc>>, _>("daily_reset")
                        .map(Into::into),
                    weekly_resets_at: row
                        .get::<Option<DateTime<Utc>>, _>("weekly_reset")
                        .map(Into::into),
                },
                key_count: number("key_count")?,
            });
        }
        tx.commit().await.map_err(unavailable)?;
        Ok(records)
    }

    pub async fn save(
        &self,
        policy: UserPolicyUpdate,
        initial_hash: Option<&str>,
    ) -> AdminStoreResult<Revision> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        // 与其它配置写入使用同一锁，保证最后一个管理员检查及快照版本原子提交。
        let revision: i64 = sqlx::query_scalar("update runtime_settings set config_revision=config_revision+1, updated_at=now() where id=1 returning config_revision").fetch_one(&mut *tx).await.map_err(unavailable)?;
        if let Some(hash) = initial_hash {
            sqlx::query("insert into admin_users(id,password_hash,role,enabled,all_groups,created_at,updated_at) values($1,$2,$3,$4,$5,now(),now())")
                .bind(&policy.id).bind(hash).bind(policy.role.as_str()).bind(policy.enabled).bind(policy.all_groups).execute(&mut *tx).await.map_err(write_error)?;
        } else {
            let exists: bool = sqlx::query_scalar(
                "select exists(select 1 from admin_users where id=$1 and deleted_at is null)",
            )
            .bind(&policy.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(unavailable)?;
            if !exists {
                return Err(AdminStoreError::new(
                    AdminStoreErrorKind::NotFound,
                    "user",
                    "user not found",
                ));
            }
            if policy.role != UserRole::Admin || !policy.enabled {
                let other: bool = sqlx::query_scalar("select exists(select 1 from admin_users where role='admin' and enabled and deleted_at is null and id<>$1)").bind(&policy.id).fetch_one(&mut *tx).await.map_err(unavailable)?;
                if !other {
                    return Err(AdminStoreError::new(
                        AdminStoreErrorKind::Conflict,
                        "user",
                        "last enabled administrator",
                    ));
                }
            }
        }
        sqlx::query("update admin_users set auth_revision=auth_revision+case when enabled<>$2 or role<>$3 then 1 else 0 end, enabled=$2, role=$3, all_groups=$4, daily_limit_usd=$5::text::numeric, weekly_limit_usd=$6::text::numeric, max_concurrency=$7, requests_per_minute=$8, updated_at=now() where id=$1")
            .bind(&policy.id).bind(policy.enabled).bind(policy.role.as_str()).bind(policy.all_groups).bind(policy.budget.daily_usd.canonical()).bind(policy.budget.weekly_usd.canonical())
            .bind(i64::try_from(policy.limits.max_concurrency).map_err(|_| invalid("limit overflow"))?).bind(i64::try_from(policy.limits.requests_per_minute).map_err(|_| invalid("limit overflow"))?).execute(&mut *tx).await.map_err(write_error)?;
        sqlx::query("delete from user_account_groups where user_id=$1")
            .bind(&policy.id)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        for group in policy.group_ids {
            sqlx::query("insert into user_account_groups(user_id,account_group_id) values($1,$2) on conflict do nothing").bind(&policy.id).bind(group.as_str()).execute(&mut *tx).await.map_err(write_error)?;
        }
        tx.commit().await.map_err(unavailable)?;
        Revision::new(u64::try_from(revision).map_err(|_| invalid("revision overflow"))?)
            .map_err(|_| invalid("invalid revision"))
    }

    pub async fn change_password(
        &self,
        id: &str,
        expected_hash: Option<&str>,
        new_hash: &str,
    ) -> AdminStoreResult<bool> {
        let result = sqlx::query("update admin_users set password_hash=$3,auth_revision=auth_revision+1,updated_at=now() where id=$1 and deleted_at is null and ($2::text is null or (enabled and password_hash=$2))")
            .bind(id).bind(expected_hash).bind(new_hash).execute(&self.pool).await.map_err(unavailable)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete(&self, id: &str) -> AdminStoreResult<Revision> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        // 配置锁串行化删除与角色修改；用户行先于 Key 锁定，与费用结算保持相同顺序。
        let revision = super::bump_config_revision_in_transaction(&mut tx)
            .await
            .map_err(|error| crate::admin_store_error("user", error))?;
        let role = sqlx::query_scalar::<_, String>(
            "select role from admin_users where id=$1 and deleted_at is null for no key update",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(unavailable)?
        .ok_or_else(|| {
            AdminStoreError::new(AdminStoreErrorKind::NotFound, "user", "user not found")
        })?;
        if role == "admin" {
            let other: bool = sqlx::query_scalar(
                "select exists(select 1 from admin_users where role='admin' and enabled and deleted_at is null and id<>$1)",
            ).bind(id).fetch_one(&mut *tx).await.map_err(unavailable)?;
            if !other {
                return Err(AdminStoreError::new(
                    AdminStoreErrorKind::Conflict,
                    "user",
                    "last enabled administrator",
                ));
            }
        }
        sqlx::query("update admin_users set enabled=false, auth_revision=auth_revision+1, deleted_at=now(), updated_at=now() where id=$1")
            .bind(id).execute(&mut *tx).await.map_err(unavailable)?;
        sqlx::query("delete from client_api_keys where user_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("delete from user_account_groups where user_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        crate::admin_revision(revision)
    }
}

fn user_identity(row: &sqlx::postgres::PgRow) -> AdminStoreResult<UserIdentity> {
    Ok(UserIdentity {
        id: row.get("id"),
        role: match row.get::<&str, _>("role") {
            "admin" => UserRole::Admin,
            "user" => UserRole::User,
            _ => return Err(invalid("invalid role")),
        },
        enabled: row.get("enabled"),
        auth_revision: row.get("auth_revision"),
    })
}

fn invalid(message: &str) -> AdminStoreError {
    AdminStoreError::new(AdminStoreErrorKind::Invalid, "user", message)
}
fn unavailable(_: sqlx::Error) -> AdminStoreError {
    AdminStoreError::new(
        AdminStoreErrorKind::Unavailable,
        "user",
        "user storage unavailable",
    )
}
fn write_error(error: sqlx::Error) -> AdminStoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AdminStoreError::new(AdminStoreErrorKind::Conflict, "user", "user already exists")
        }
        sqlx::Error::Database(db) if db.is_foreign_key_violation() || db.is_check_violation() => {
            invalid("invalid user policy")
        }
        _ => unavailable(error),
    }
}
