//! 个人统计复用观测查询和展示映射；归属始终来自会话，禁止账号维度与运维原文。

use super::*;
use crate::admin::users::UserAuth;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersonalUsageQuery {
    start_time: Option<String>,
    end_time: Option<String>,
    current_page: Option<u32>,
    page_size: Option<u16>,
    provider: Option<String>,
    model: Option<String>,
    status_code: Option<i64>,
    search: Option<String>,
    dimension: Option<String>,
}

impl PersonalUsageQuery {
    fn command(&self, user_id: String) -> Result<domain::UsageQuery, AdminError> {
        let mut command = usage_command(&UsageQuery {
            start_time: self.start_time.clone(),
            end_time: self.end_time.clone(),
            current_page: self.current_page,
            page_size: self.page_size,
            provider: self.provider.clone(),
            model: self.model.clone(),
            status_code: self.status_code,
            search: self.search.clone(),
            ..Default::default()
        })
        .map_err(map_wire_error)?;
        command.filter.user_id = Some(user_id);
        Ok(command)
    }
}

pub(super) fn router<S: AdminSessionState + Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/api/user/usage/records", get(records::<S>))
        .route("/api/user/usage/records/summary", get(summary::<S>))
        .route("/api/user/usage/insights/overview", get(overview::<S>))
        .route(
            "/api/user/usage/insights/diagnostics",
            get(diagnostics::<S>),
        )
}

async fn records<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<PersonalUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let result = state
        .admin_services()
        .observability()
        .usage_records(query.command(auth.0.id)?)
        .await
        .map_err(map_service_error)?;
    let mut data = usage_page_view(result);
    for row in &mut data.items {
        row.account_id = None;
        row.account_name = None;
        row.account_email = None;
        row.username = None;
        row.authentication_kind = None;
        row.latency_details.capacity_used_slots = None;
        row.latency_details.capacity_total_slots = None;
    }
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(data)))
}

async fn summary<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<PersonalUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let command = query.command(auth.0.id)?;
    let result = state
        .admin_services()
        .observability()
        .usage_summary(command.range, command.filter)
        .await
        .map_err(map_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(usage_summary_view(result)),
    ))
}

async fn overview<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<PersonalUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let command = query.command(auth.0.id)?;
    let result = state
        .admin_services()
        .observability()
        .usage_insights(command.range, command.filter)
        .await
        .map_err(map_service_error)?;
    let mut view = usage_insights_view(result);
    // 账号容量是全局运行信息，不通过个人请求的采样间接公开。
    view.performance.capacity_utilization = None;
    view.performance.capacity_utilization_p95 = None;
    view.performance.capacity_coverage = 0.0;
    for point in &mut view.performance.points {
        point.capacity_utilization = None;
        point.capacity_utilization_p95 = None;
    }
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(view)))
}

async fn diagnostics<S: AdminSessionState + Send + Sync>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<PersonalUsageQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let dimension =
        DiagnosticDimension::parse(query.dimension.as_deref()).map_err(map_wire_error)?;
    if matches!(dimension, DiagnosticDimension::Account) {
        return Err(AdminError::bad_request("个人统计不支持账号维度"));
    }
    let command = query.command(auth.0.id)?;
    let result = state
        .admin_services()
        .observability()
        .diagnostics(
            command.range,
            command.filter,
            domain_diagnostic_dimension(dimension),
        )
        .await
        .map_err(map_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(diagnostics_view(result, dimension)),
    ))
}
