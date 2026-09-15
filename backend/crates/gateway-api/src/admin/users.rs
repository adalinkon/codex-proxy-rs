//! 用户管理与自助面板的窄 HTTP 合同；用户 ID 只由已验证会话注入。

use super::{
    AdminAuth, AdminEnvelope, AdminError, AdminJson, AdminQuery, AdminResponse, AdminSessionState,
    auth::admin_session_cookie,
    client_keys::{
        ClientKeyView, CreateClientKeyRequest, CreatedClientKeyData, ListClientKeysQuery,
        RevealedClientKeyData, encode_page_cursor,
    },
    wire::map_admin_service_error,
};
use axum::{
    Router,
    extract::{FromRequestParts, State},
    http::{StatusCode, request::Parts},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use gateway_admin::model::{
    PageSize,
    observability::{TimeRange, UsageFilter, UsageQuery},
    users::{
        RequestUsage, RequestUsageScope, UserIdentity, UserPolicyUpdate, UserRecord, UserRole,
    },
};
use gateway_core::{
    engine::budget::ClientBudgetStatus,
    policy::{ClientApiKeyId, RateLimits},
    routing::AccountGroupId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUserView {
    id: String,
    username: String,
    role: &'static str,
}
impl From<UserIdentity> for SessionUserView {
    fn from(user: UserIdentity) -> Self {
        Self {
            username: user.id.clone(),
            id: user.id,
            role: user.role.as_str(),
        }
    }
}

pub struct UserAuth(pub(crate) UserIdentity);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnedKeyPage {
    items: Vec<ClientKeyView>,
    total: u64,
    next_cursor: Option<String>,
}
impl<S: AdminSessionState + Send + Sync> FromRequestParts<S> for UserAuth {
    type Rejection = AdminError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, AdminError> {
        state
            .admin_services()
            .auth()
            .resolve_user(admin_session_cookie(&parts.headers).as_deref())
            .await
            .map_err(map_admin_service_error)?
            .map(Self)
            .ok_or_else(AdminError::admin_session_required)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetView {
    daily_limit_usd: String,
    weekly_limit_usd: String,
    daily_used_usd: String,
    weekly_used_usd: String,
    daily_remaining_usd: Option<String>,
    weekly_remaining_usd: Option<String>,
    daily_resets_at: Option<DateTime<Utc>>,
    weekly_resets_at: Option<DateTime<Utc>>,
}
impl From<ClientBudgetStatus> for BudgetView {
    fn from(b: ClientBudgetStatus) -> Self {
        let remaining = |limit: gateway_core::metering::Decimal,
                         used: gateway_core::metering::Decimal| {
            if limit == gateway_core::metering::Decimal::ZERO {
                None
            } else {
                Some(
                    gateway_core::metering::Decimal::from_scaled(
                        limit.scaled().saturating_sub(used.scaled()),
                    )
                    .unwrap_or_default()
                    .canonical(),
                )
            }
        };
        Self {
            daily_limit_usd: b.limits.daily_usd.canonical(),
            weekly_limit_usd: b.limits.weekly_usd.canonical(),
            daily_used_usd: b.daily_used_usd.canonical(),
            weekly_used_usd: b.weekly_used_usd.canonical(),
            daily_remaining_usd: remaining(b.limits.daily_usd, b.daily_used_usd),
            weekly_remaining_usd: remaining(b.limits.weekly_usd, b.weekly_used_usd),
            daily_resets_at: b.daily_resets_at.map(Into::into),
            weekly_resets_at: b.weekly_resets_at.map(Into::into),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupView {
    id: String,
    name: String,
    color: String,
    enabled: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserView {
    #[serde(flatten)]
    identity: SessionUserView,
    enabled: bool,
    groups: Vec<GroupView>,
    max_concurrency: u64,
    requests_per_minute: u64,
    key_count: u64,
    #[serde(flatten)]
    budget: BudgetView,
}
impl From<UserRecord> for UserView {
    fn from(u: UserRecord) -> Self {
        Self {
            enabled: u.identity.enabled,
            identity: u.identity.into(),
            groups: u
                .groups
                .into_iter()
                .map(|g| GroupView {
                    id: g.id.to_string(),
                    name: g.name,
                    color: g.color.as_str().to_owned(),
                    enabled: g.enabled,
                })
                .collect(),
            max_concurrency: u.limits.max_concurrency,
            requests_per_minute: u.limits.requests_per_minute,
            key_count: u.key_count,
            budget: u.budget.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveUser {
    username: String,
    role: String,
    enabled: bool,
    group_ids: Vec<String>,
    daily_limit_usd: String,
    weekly_limit_usd: String,
    max_concurrency: u64,
    requests_per_minute: u64,
    password: Option<String>,
}
impl SaveUser {
    fn policy(&self) -> Result<UserPolicyUpdate, AdminError> {
        let invalid = || AdminError::bad_request("用户配置不合法");
        Ok(UserPolicyUpdate {
            id: self.username.clone(),
            role: match self.role.as_str() {
                "admin" => UserRole::Admin,
                "user" => UserRole::User,
                _ => return Err(invalid()),
            },
            enabled: self.enabled,
            group_ids: self
                .group_ids
                .iter()
                .map(|id| AccountGroupId::new(id.clone()).map_err(|_| invalid()))
                .collect::<Result<_, _>>()?,
            limits: RateLimits {
                max_concurrency: self.max_concurrency,
                requests_per_minute: self.requests_per_minute,
            },
            budget: gateway_core::engine::budget::ClientBudgetLimits {
                daily_usd: self.daily_limit_usd.parse().map_err(|_| invalid())?,
                weekly_usd: self.weekly_limit_usd.parse().map_err(|_| invalid())?,
            },
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResetPassword {
    id: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteUser {
    id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResetBudget {
    id: String,
    operation_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetBudgetResult {
    reset_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestUsageQuery {
    ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestUsageView {
    id: String,
    current_concurrency: Option<u64>,
    current_rpm: Option<u64>,
}
impl From<RequestUsage> for RequestUsageView {
    fn from(value: RequestUsage) -> Self {
        Self {
            id: value.id,
            current_concurrency: value.current_concurrency,
            current_rpm: value.current_rpm,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChangePassword {
    current_password: String,
    new_password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateKeyIdentity {
    id: String,
    name: String,
    label: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyId {
    id: String,
}
impl KeyId {
    fn parsed(self) -> Result<ClientApiKeyId, AdminError> {
        ClientApiKeyId::new(self.id).map_err(|_| AdminError::bad_request("Key ID 不合法"))
    }
}

pub fn router<S: AdminSessionState + Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/api/admin/users", get(list_users::<S>))
        .route("/api/admin/users/create", post(create_user::<S>))
        .route("/api/admin/users/update", post(update_user::<S>))
        .route("/api/admin/users/reset-password", post(reset_password::<S>))
        .route("/api/admin/users/delete", post(delete_user::<S>))
        .route("/api/admin/users/reset-budget", post(reset_budget::<S>))
        .route(
            "/api/admin/users/request-usage",
            post(user_request_usage::<S>),
        )
        .route(
            "/api/admin/client-keys/request-usage",
            post(key_request_usage::<S>),
        )
        .route("/api/user/request-usage", get(my_request_usage::<S>))
        .route(
            "/api/user/client-keys/request-usage",
            post(my_key_request_usage::<S>),
        )
        .route("/api/user/profile", get(profile::<S>))
        .route("/api/user/password", post(change_password::<S>))
        .route("/api/user/client-keys", get(keys::<S>))
        .route("/api/user/client-keys/create", post(create_key::<S>))
        .route("/api/user/client-keys/update", post(update_key::<S>))
        .route("/api/user/client-keys/reveal", get(reveal_key::<S>))
        .route("/api/user/client-keys/enable", post(enable_key::<S>))
        .route("/api/user/client-keys/disable", post(disable_key::<S>))
        .route("/api/user/client-keys/delete", post(delete_key::<S>))
        .route("/api/user/usage", get(usage::<S>))
}

async fn delete_user<S: AdminSessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminJson(command): AdminJson<DeleteUser>,
) -> Result<impl IntoResponse, AdminError> {
    state
        .admin_services()
        .users()
        .delete(&command.id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}

async fn usage_response<S: AdminSessionState + Send + Sync>(
    s: S,
    scope: RequestUsageScope,
    ids: Vec<String>,
) -> Result<impl IntoResponse, AdminError> {
    let result = s
        .admin_services()
        .users()
        .request_usage(scope, ids)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(
            result
                .into_iter()
                .map(RequestUsageView::from)
                .collect::<Vec<_>>(),
        ),
    ))
}
async fn user_request_usage<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<RequestUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    usage_response(s, RequestUsageScope::Users, p.ids).await
}
async fn key_request_usage<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<RequestUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    usage_response(s, RequestUsageScope::Keys { user_id: None }, p.ids).await
}
async fn my_request_usage<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
) -> Result<impl IntoResponse, AdminError> {
    usage_response(s, RequestUsageScope::Users, vec![auth.0.id]).await
}
async fn my_key_request_usage<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<RequestUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    usage_response(
        s,
        RequestUsageScope::Keys {
            user_id: Some(auth.0.id),
        },
        p.ids,
    )
    .await
}

async fn reset_budget<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(state): State<S>,
    AdminJson(command): AdminJson<ResetBudget>,
) -> Result<impl IntoResponse, AdminError> {
    let reset_at = state
        .admin_services()
        .users()
        .reset_budget(&command.id, &command.operation_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(ResetBudgetResult { reset_at }),
    ))
}

async fn list_users<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(s): State<S>,
) -> Result<impl IntoResponse, AdminError> {
    let items = s
        .admin_services()
        .users()
        .list()
        .await
        .map_err(map_admin_service_error)?
        .into_iter()
        .map(UserView::from)
        .collect::<Vec<_>>();
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(items)))
}
async fn create_user<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<SaveUser>,
) -> Result<impl IntoResponse, AdminError> {
    let policy = p.policy()?;
    let password = p
        .password
        .ok_or_else(|| AdminError::bad_request("必须设置初始密码"))?;
    let user = s
        .admin_services()
        .users()
        .save(policy, Some(password))
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::CREATED,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}
async fn update_user<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<SaveUser>,
) -> Result<impl IntoResponse, AdminError> {
    if p.password.is_some() {
        return Err(AdminError::bad_request("请使用重置密码接口"));
    }
    let user = s
        .admin_services()
        .users()
        .save(p.policy()?, None)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}
async fn reset_password<S: AdminSessionState + Send + Sync>(
    _: AdminAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<ResetPassword>,
) -> Result<impl IntoResponse, AdminError> {
    s.admin_services()
        .users()
        .reset_password(&p.id, &p.password)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}
async fn profile<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
) -> Result<impl IntoResponse, AdminError> {
    let user = s
        .admin_services()
        .users()
        .get(&auth.0.id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}
async fn change_password<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<ChangePassword>,
) -> Result<impl IntoResponse, AdminError> {
    s.admin_services()
        .auth()
        .change_password(&auth.0.id, &p.current_password, &p.new_password)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}
async fn keys<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminQuery(p): AdminQuery<ListClientKeysQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let mut query = p
        .into_command()
        .map_err(|_| AdminError::bad_request("查询不合法"))?;
    query.user_id = Some(auth.0.id);
    let page = s
        .admin_services()
        .client_keys()
        .list(query)
        .await
        .map_err(map_admin_service_error)?;
    let data = OwnedKeyPage {
        next_cursor: encode_page_cursor(page.next_cursor).map_err(|_| AdminError::internal())?,
        total: page.total,
        items: page.items.into_iter().map(ClientKeyView::owned).collect(),
    };
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(data)))
}
async fn create_key<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<CreateClientKeyRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let command = p
        .into_command()
        .map_err(super::client_keys::map_wire_error)?;
    if command.user_id.is_some() {
        return Err(AdminError::bad_request("个人 Key 不接受指定所属用户"));
    }
    let key = s
        .admin_services()
        .client_keys()
        .create_owned(&auth.0.id, command)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::CREATED,
        AdminEnvelope::ok(CreatedClientKeyData::from(key)),
    ))
}
async fn update_key<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<UpdateKeyIdentity>,
) -> Result<impl IntoResponse, AdminError> {
    let id = KeyId { id: p.id }.parsed()?;
    s.admin_services()
        .client_keys()
        .update_owned_identity(&auth.0.id, id, p.name, p.label)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}
async fn reveal_key<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminQuery(p): AdminQuery<KeyId>,
) -> Result<impl IntoResponse, AdminError> {
    let key = s
        .admin_services()
        .client_keys()
        .reveal_owned(&auth.0.id, &p.parsed()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(RevealedClientKeyData::from(key)),
    ))
}
async fn own_key_mutation<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    s: S,
    p: KeyId,
    enabled: Option<bool>,
) -> Result<impl IntoResponse, AdminError> {
    s.admin_services()
        .client_keys()
        .mutate_owned(&auth.0.id, p.parsed()?, enabled)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}
async fn enable_key<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<KeyId>,
) -> Result<impl IntoResponse, AdminError> {
    own_key_mutation(auth, s, p, Some(true)).await
}
async fn disable_key<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<KeyId>,
) -> Result<impl IntoResponse, AdminError> {
    own_key_mutation(auth, s, p, Some(false)).await
}
async fn delete_key<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminJson(p): AdminJson<KeyId>,
) -> Result<impl IntoResponse, AdminError> {
    own_key_mutation(auth, s, p, None).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserUsageQuery {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    page: Option<u32>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsageRow {
    id: String,
    model: Option<String>,
    started_at: DateTime<Utc>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    cost_usd: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsagePage {
    items: Vec<UserUsageRow>,
    total: u64,
    page: u32,
    request_count: u64,
    total_tokens: u64,
    cost_usd: Option<String>,
}
async fn usage<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(s): State<S>,
    AdminQuery(p): AdminQuery<UserUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let range =
        TimeRange::new(p.start, p.end).map_err(|_| AdminError::bad_request("时间范围不合法"))?;
    let page = p.page.unwrap_or(1);
    if page == 0 || page > 100_000 {
        return Err(AdminError::bad_request("页码不合法"));
    }
    let filter = UsageFilter {
        user_id: Some(auth.0.id),
        ..Default::default()
    };
    let summary = s
        .admin_services()
        .observability()
        .usage_summary(range, filter.clone())
        .await
        .map_err(map_admin_service_error)?;
    let result = s
        .admin_services()
        .observability()
        .usage_records(UsageQuery {
            range,
            filter,
            current_page: page,
            page_size: PageSize::new(50).map_err(|_| AdminError::internal())?,
        })
        .await
        .map_err(map_admin_service_error)?;
    let items = result
        .items
        .into_iter()
        .map(|r| UserUsageRow {
            id: r.id,
            model: r.requested_model_id,
            started_at: r.started_at,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            total_tokens: r.total_tokens,
            cost_usd: r
                .cost_amount
                .filter(|_| r.cost_currency.as_deref() == Some("USD"))
                .map(|amount| amount.as_str().to_owned()),
        })
        .collect();
    let cost_usd = summary
        .overview
        .attempts
        .costs
        .into_iter()
        .find(|cost| cost.currency == "USD")
        .map(|cost| cost.amount.as_str().to_owned());
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserUsagePage {
            items,
            total: result.total,
            page,
            request_count: summary.overview.requests.request_count,
            total_tokens: summary.overview.requests.total_tokens,
            cost_usd,
        }),
    ))
}
