//! 用户会话的 Redis 存储，保留认证版本失效机制。

use super::{MAX_REDIS_EXACT_INTEGER, namespace, resource_fingerprint};
use crate::{StoreError, StoreResult, redis_unavailable, require_nonempty};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};

/// Redis 中可丢失的管理员会话事实；认证秘密不属于该结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSessionRecord {
    pub admin_user_id: String,
    pub auth_revision: i64,
    pub expires_at: DateTime<Utc>,
}

impl AdminSessionRecord {
    fn validate(&self) -> StoreResult<u64> {
        require_nonempty("admin session", "admin_user_id", &self.admin_user_id)?;
        let expires_at_millis = u64::try_from(self.expires_at.timestamp_millis())
            .map_err(|_| admin_auth_invalid("session expiry must be after the Unix epoch"))?;
        if expires_at_millis > MAX_REDIS_EXACT_INTEGER {
            return Err(admin_auth_invalid(
                "session expiry is outside the supported range",
            ));
        }
        let now_millis = u64::try_from(Utc::now().timestamp_millis())
            .map_err(|_| admin_auth_invalid("current time is outside the supported range"))?;
        if expires_at_millis <= now_millis {
            return Err(admin_auth_invalid("session expiry must be in the future"));
        }
        Ok(expires_at_millis)
    }
}

/// 管理员会话的 Redis 基础设施端口。
#[async_trait]
pub trait AdminAuthStateRepository: Send + Sync {
    async fn load_admin_session(&self, session_id: &str)
    -> StoreResult<Option<AdminSessionRecord>>;
    async fn store_admin_session(
        &self,
        session_id: &str,
        session: &AdminSessionRecord,
    ) -> StoreResult<()>;
    async fn delete_admin_session(
        &self,
        session_id: &str,
    ) -> StoreResult<Option<AdminSessionRecord>>;
}

/// Redis 管理员会话 adapter。
#[derive(Clone)]
pub struct RedisAdminAuthStateRepository {
    connection: ConnectionManager,
    namespace: String,
}

impl RedisAdminAuthStateRepository {
    pub fn new(connection: ConnectionManager, key_namespace: &str) -> StoreResult<Self> {
        Ok(Self {
            connection,
            namespace: format!("{}:admin-auth:v1", namespace(key_namespace)?),
        })
    }

    fn session_key(&self, session_id: &str) -> StoreResult<String> {
        let fingerprint = resource_fingerprint("admin session", session_id)?;
        Ok(format!("{}:session:{{{fingerprint}}}", self.namespace))
    }
}

#[async_trait]
impl AdminAuthStateRepository for RedisAdminAuthStateRepository {
    async fn load_admin_session(
        &self,
        session_id: &str,
    ) -> StoreResult<Option<AdminSessionRecord>> {
        let key = self.session_key(session_id)?;
        let mut connection = self.connection.clone();
        let payload = redis::cmd("GET")
            .arg(key)
            .query_async::<Option<String>>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("load admin session"))?;
        payload
            .map(|value| decode_admin_session(&value))
            .transpose()
    }

    async fn store_admin_session(
        &self,
        session_id: &str,
        session: &AdminSessionRecord,
    ) -> StoreResult<()> {
        let key = self.session_key(session_id)?;
        let expires_at_millis = session.validate()?;
        let payload = encode_admin_session(session)?;
        let mut connection = self.connection.clone();
        redis::cmd("SET")
            .arg(key)
            .arg(payload)
            .arg("PXAT")
            .arg(expires_at_millis)
            .query_async::<String>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("store admin session"))?;
        Ok(())
    }

    async fn delete_admin_session(
        &self,
        session_id: &str,
    ) -> StoreResult<Option<AdminSessionRecord>> {
        let key = self.session_key(session_id)?;
        let mut connection = self.connection.clone();
        let payload = redis::cmd("GETDEL")
            .arg(key)
            .query_async::<Option<String>>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("delete admin session"))?;
        payload
            .map(|value| decode_admin_session(&value))
            .transpose()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminSessionWire {
    admin_user_id: String,
    #[serde(default)]
    auth_revision: i64,
    expires_at: String,
}

fn encode_admin_session(session: &AdminSessionRecord) -> StoreResult<String> {
    serde_json::to_string(&AdminSessionWire {
        admin_user_id: session.admin_user_id.clone(),
        auth_revision: session.auth_revision,
        expires_at: session
            .expires_at
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
    })
    .map_err(|_| admin_auth_invalid("session value cannot be encoded"))
}

fn decode_admin_session(value: &str) -> StoreResult<AdminSessionRecord> {
    let wire: AdminSessionWire = serde_json::from_str(value)
        .map_err(|_| admin_auth_invalid("Redis returned an invalid session value"))?;
    require_nonempty("admin session", "admin_user_id", &wire.admin_user_id)?;
    let expires_at = DateTime::parse_from_rfc3339(&wire.expires_at)
        .map_err(|_| admin_auth_invalid("Redis returned an invalid session expiry"))?
        .with_timezone(&Utc);
    Ok(AdminSessionRecord {
        admin_user_id: wire.admin_user_id,
        auth_revision: wire.auth_revision,
        expires_at,
    })
}

fn admin_auth_invalid(message: &str) -> StoreError {
    StoreError::InvalidData {
        entity: "admin authentication state",
        message: message.to_owned(),
    }
}
