//! 管理员维护用户；不新增操作审计。

use super::{
    auth::{hash_admin_password, validate_password},
    map_store_error, publish_committed,
};
use crate::{
    model::{
        AdminError,
        users::{RequestUsage, RequestUsageScope, UserPolicyUpdate, UserRecord},
    },
    ports::store::{AuthStore, RequestUsageStore},
};
use async_trait::async_trait;
use gateway_core::runtime::SnapshotControl;
use std::sync::Arc;

#[async_trait]
pub trait UserService: Send + Sync {
    async fn request_usage(
        &self,
        scope: RequestUsageScope,
        ids: Vec<String>,
    ) -> Result<Vec<RequestUsage>, AdminError>;
    async fn list(&self) -> Result<Vec<UserRecord>, AdminError>;
    async fn get(&self, id: &str) -> Result<UserRecord, AdminError>;
    async fn save(
        &self,
        policy: UserPolicyUpdate,
        password: Option<String>,
    ) -> Result<UserRecord, AdminError>;
    async fn reset_password(&self, id: &str, password: &str) -> Result<(), AdminError>;
    async fn delete(&self, id: &str) -> Result<(), AdminError>;
    async fn reset_budget(
        &self,
        id: &str,
        operation_id: &str,
    ) -> Result<chrono::DateTime<chrono::Utc>, AdminError>;
}

pub(crate) struct DefaultUserService {
    request_usage: Option<Arc<dyn RequestUsageStore>>,
    store: Arc<dyn AuthStore>,
    snapshot: Arc<dyn SnapshotControl>,
}
impl DefaultUserService {
    pub(crate) fn new(
        store: Arc<dyn AuthStore>,
        snapshot: Arc<dyn SnapshotControl>,
        request_usage: Option<Arc<dyn RequestUsageStore>>,
    ) -> Self {
        Self {
            store,
            snapshot,
            request_usage,
        }
    }
}

#[async_trait]
impl UserService for DefaultUserService {
    async fn request_usage(
        &self,
        scope: RequestUsageScope,
        mut ids: Vec<String>,
    ) -> Result<Vec<RequestUsage>, AdminError> {
        if ids.len() > 100
            || ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err(AdminError::invalid("实时请求计数查询不合法"));
        }
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let store = self
            .request_usage
            .as_ref()
            .ok_or_else(|| AdminError::internal("实时请求计数暂不可用"))?;
        store
            .request_usage(scope, ids)
            .await
            .map_err(|error| map_store_error(error, "request usage"))
    }
    async fn reset_budget(
        &self,
        id: &str,
        operation_id: &str,
    ) -> Result<chrono::DateTime<chrono::Utc>, AdminError> {
        let operation_id = uuid::Uuid::parse_str(operation_id)
            .map_err(|_| AdminError::invalid("重置操作 ID 不合法"))?
            .to_string();
        self.store
            .reset_user_budget(id, &operation_id)
            .await
            .map_err(|error| map_store_error(error, "user budget"))
    }
    async fn delete(&self, id: &str) -> Result<(), AdminError> {
        let revision = self
            .store
            .delete_user(id)
            .await
            .map_err(|error| map_store_error(error, "user"))?;
        publish_committed(self.snapshot.as_ref(), revision).await
    }
    async fn list(&self) -> Result<Vec<UserRecord>, AdminError> {
        self.store
            .list_users()
            .await
            .map_err(|e| map_store_error(e, "user"))
    }
    async fn get(&self, id: &str) -> Result<UserRecord, AdminError> {
        self.store
            .load_user(id)
            .await
            .map_err(|e| map_store_error(e, "user"))?
            .ok_or_else(|| AdminError::not_found("用户不存在"))
    }
    async fn save(
        &self,
        policy: UserPolicyUpdate,
        password: Option<String>,
    ) -> Result<UserRecord, AdminError> {
        if policy.id.trim() != policy.id
            || policy.id.is_empty()
            || policy.id.len() > 128
            || policy.id.chars().any(char::is_control)
        {
            return Err(AdminError::invalid("用户名不合法"));
        }
        if policy.limits.max_concurrency >= (1_u64 << 53)
            || policy.limits.requests_per_minute >= (1_u64 << 53)
        {
            return Err(AdminError::invalid("请求限制超出支持范围"));
        }
        let hash = password
            .as_deref()
            .map(|password| {
                validate_password(password)?;
                hash_admin_password(password)
            })
            .transpose()?;
        let id = policy.id.clone();
        let revision = self
            .store
            .save_user(policy, hash.as_deref())
            .await
            .map_err(|e| map_store_error(e, "user"))?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        self.get(&id).await
    }
    async fn reset_password(&self, id: &str, password: &str) -> Result<(), AdminError> {
        validate_password(password)?;
        let hash = hash_admin_password(password)?;
        if !self
            .store
            .change_password(id, None, &hash)
            .await
            .map_err(|e| map_store_error(e, "user"))?
        {
            return Err(AdminError::not_found("用户不存在"));
        }
        Ok(())
    }
}
