use super::AdminTestFixture;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

pub(super) struct TestRequestUsageStore;

#[tokio::test]
async fn personal_key_creation_rejects_explicit_owner() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("administrator");
    let response = call(
        &fixture,
        "POST",
        "/api/user/client-keys/create",
        "administrator",
        json!({
            "name": "Personal", "label": "Test", "userId": "another-user",
            "groupIds": [], "maxConcurrency": 2, "requestsPerMinute": 10,
            "dailyLimitUsd": "1", "weeklyLimitUsd": "5"
        }),
    )
    .await;
    assert_eq!(response.0, StatusCode::BAD_REQUEST);
}

#[async_trait::async_trait]
impl gateway_admin::ports::store::RequestUsageStore for TestRequestUsageStore {
    async fn request_usage(
        &self,
        scope: gateway_admin::model::users::RequestUsageScope,
        ids: Vec<String>,
    ) -> gateway_admin::ports::store::AdminStoreResult<Vec<gateway_admin::model::users::RequestUsage>>
    {
        use gateway_admin::model::users::{RequestUsage, RequestUsageScope};
        if let RequestUsageScope::Keys {
            user_id: Some(owner),
        } = scope
        {
            assert_eq!(owner, "alice", "个人 Key 查询必须传入当前会话用户");
        }
        Ok(ids
            .into_iter()
            .map(|id| RequestUsage {
                id,
                current_concurrency: Some(2),
                current_rpm: Some(7),
            })
            .collect())
    }
}

#[tokio::test]
async fn realtime_request_usage_routes_bind_personal_scope_and_limit_batch_size() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("administrator");
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/create",
            "administrator",
            user_payload()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let session = login(&fixture, "alice-initial-password").await;
    let own = call(
        &fixture,
        "GET",
        "/api/user/request-usage?userId=bob",
        &session,
        Value::Null,
    )
    .await;
    assert_eq!(own.0, StatusCode::OK);
    assert_eq!(own.1["data"][0]["id"], "alice");
    assert_eq!(own.1["data"][0]["currentConcurrency"], 2);
    assert_eq!(own.1["data"][0]["currentRpm"], 7);
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/user/client-keys/request-usage",
            &session,
            json!({"ids":["own-key"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/user/client-keys/request-usage",
            &session,
            json!({"ids":["own-key"],"userId":"bob"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for path in [
        "/api/admin/users/request-usage",
        "/api/admin/client-keys/request-usage",
    ] {
        assert_eq!(
            call(&fixture, "POST", path, &session, json!({"ids":["alice"]}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                &fixture,
                "POST",
                path,
                "administrator",
                json!({"ids":vec!["a";101]})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        let response = call(
            &fixture,
            "POST",
            path,
            "administrator",
            json!({"ids":["alice","alice"]}),
        )
        .await;
        assert_eq!(response.0, StatusCode::OK);
        assert_eq!(response.1["data"].as_array().unwrap().len(), 1);
    }
    assert_eq!(
        call(
            &fixture,
            "GET",
            "/api/user/request-usage",
            "missing",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

async fn call(
    fixture: &AdminTestFixture,
    method: &str,
    path: &str,
    session: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = gateway_api::admin::router()
        .with_state(fixture.state())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::COOKIE, format!("cpr_admin_session={session}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-request-id", "req_user_test")
                .body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn user_payload() -> Value {
    json!({"username":"alice","role":"user","enabled":true,"groupIds":[],"dailyLimitUsd":"1.1234567891","weeklyLimitUsd":"7","maxConcurrency":2,"requestsPerMinute":10,"password":"alice-initial-password"})
}

#[tokio::test]
async fn budget_reset_requires_admin_and_rejects_invalid_commands_without_changing_policy() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("administrator");
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/create",
            "administrator",
            user_payload()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let session = login(&fixture, "alice-initial-password").await;
    let payload = json!({"id":"alice","operationId":"5fb66506-b594-45f0-923a-a7ff0533a11b"});
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/reset-budget",
            &session,
            payload.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/reset-budget",
            "unknown",
            payload.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/reset-budget",
            "administrator",
            json!({"id":"alice","operationId":"invalid"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/reset-budget",
            "administrator",
            json!({"id":"alice"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let before = call(&fixture, "GET", "/api/user/profile", &session, Value::Null)
        .await
        .1;
    {
        let mut users = fixture.auth.users.lock().unwrap();
        let user = &mut users.get_mut("alice").unwrap().0;
        user.budget.daily_used_usd = "0.5".parse().unwrap();
        user.budget.weekly_used_usd = "0.8".parse().unwrap();
    }
    let response = call(
        &fixture,
        "POST",
        "/api/admin/users/reset-budget",
        "administrator",
        payload,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    assert!(response.1["data"]["resetAt"].is_string());
    let after = call(&fixture, "GET", "/api/user/profile", &session, Value::Null).await;
    assert_eq!(after.0, StatusCode::OK);
    assert_eq!(after.1["data"]["dailyUsedUsd"], "0");
    assert_eq!(after.1["data"]["weeklyUsedUsd"], "0");
    for field in [
        "dailyLimitUsd",
        "weeklyLimitUsd",
        "maxConcurrency",
        "requestsPerMinute",
        "groups",
        "enabled",
    ] {
        assert_eq!(before["data"][field], after.1["data"][field]);
    }
    assert_eq!(fixture.auth.audit_count(), 0);
}

#[tokio::test]
async fn personal_statistics_bind_all_queries_to_session_and_reject_admin_fields() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("administrator");
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/create",
            "administrator",
            user_payload()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let session = login(&fixture, "alice-initial-password").await;
    for endpoint in [
        "records",
        "records/summary",
        "insights/overview",
        "insights/diagnostics",
    ] {
        let path = format!("/api/user/usage/{endpoint}?provider=openai&search=public-model");
        fixture.usage_filters.lock().unwrap().clear();
        // 汇总测试桩可能返回不可用，仍须确认进入存储前已注入归属。
        let _ = call(&fixture, "GET", &path, &session, Value::Null).await;
        {
            let filters = fixture.usage_filters.lock().unwrap();
            assert!(!filters.is_empty(), "{endpoint}");
            for filter in filters.iter() {
                assert_eq!(filter.user_id.as_deref(), Some("alice"), "{endpoint}");
                assert_eq!(filter.provider_kind.as_deref(), Some("openai"));
                assert_eq!(filter.search.as_deref(), Some("public-model"));
            }
        }
        for field in ["userId=bob", "accountId=private-account"] {
            assert_eq!(
                call(
                    &fixture,
                    "GET",
                    &format!("{path}&{field}"),
                    &session,
                    Value::Null
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            call(&fixture, "GET", &path, "missing-session", Value::Null)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &fixture,
            "GET",
            "/api/user/usage/insights/diagnostics?dimension=account",
            &session,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for path in [
        "/api/admin/usage/records",
        "/api/admin/usage/records/detail?id=other",
        "/api/admin/operations/errors",
    ] {
        assert_eq!(
            call(&fixture, "GET", path, &session, Value::Null).await.0,
            StatusCode::FORBIDDEN
        );
    }
    for policy in ["dailyLimitUsd", "groupIds", "userId"] {
        let mut payload = json!({"id":"key_1","name":"renamed","label":"personal"});
        payload[policy] = json!("forbidden");
        assert_eq!(
            call(
                &fixture,
                "POST",
                "/api/user/client-keys/update",
                &session,
                payload
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[tokio::test]
async fn user_deletion_requires_admin_and_invalidates_existing_sessions() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("administrator");
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/create",
            "administrator",
            user_payload()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let session = login(&fixture, "alice-initial-password").await;
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/delete",
            &session,
            json!({"id":"alice"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/delete",
            "administrator",
            json!({"id":"alice"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&fixture, "GET", "/api/user/profile", &session, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let listed = call(
        &fixture,
        "GET",
        "/api/admin/users",
        "administrator",
        Value::Null,
    )
    .await
    .1;
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|user| user["id"] != "alice")
    );
    assert_eq!(fixture.auth.audit_count(), 0);
}

async fn login(fixture: &AdminTestFixture, password: &str) -> String {
    let response = gateway_api::admin::router()
        .with_state(fixture.state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"alice","password":password}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("cpr_admin_session=")
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn roles_ownership_password_changes_and_disabling_are_enforced_by_backend() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("administrator");
    let (status, created) = call(
        &fixture,
        "POST",
        "/api/admin/users/create",
        "administrator",
        user_payload(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["data"]["dailyLimitUsd"], "1.1234567891");
    let first = login(&fixture, "alice-initial-password").await;
    let second = login(&fixture, "alice-initial-password").await;
    let (status, profile) = call(
        &fixture,
        "GET",
        "/api/user/profile?userId=admin_1",
        &first,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["data"]["id"], "alice");
    for field in [
        "password",
        "passwordHash",
        "authRevision",
        "accounts",
        "proxies",
    ] {
        assert!(profile["data"].get(field).is_none());
    }
    assert_eq!(
        call(&fixture, "GET", "/api/admin/users", &first, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/create",
            &first,
            user_payload()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &fixture,
            "GET",
            "/api/user/client-keys/reveal?id=key_1",
            &first,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(call(&fixture,"POST","/api/user/password",&first,json!({"currentPassword":"alice-initial-password","newPassword":"alice-updated-password","userId":"admin_1"})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(call(&fixture,"POST","/api/user/password",&first,json!({"currentPassword":"alice-initial-password","newPassword":"alice-updated-password"})).await.0,StatusCode::OK);
    for session in [&first, &second] {
        assert_eq!(
            call(&fixture, "GET", "/api/user/profile", session, Value::Null)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let current = login(&fixture, "alice-updated-password").await;
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/reset-password",
            "administrator",
            json!({"id":"alice","password":"alice-reset-password"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&fixture, "GET", "/api/user/profile", &current, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let current = login(&fixture, "alice-reset-password").await;
    let mut update = user_payload();
    update.as_object_mut().unwrap().remove("password");
    update["enabled"] = json!(false);
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/update",
            "administrator",
            update.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&fixture, "GET", "/api/user/profile", &current, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    update["enabled"] = json!(true);
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/users/update",
            "administrator",
            update
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&fixture, "GET", "/api/user/profile", &current, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let current = login(&fixture, "alice-reset-password").await;
    let status = call(
        &fixture,
        "GET",
        "/api/admin/auth/status",
        &current,
        Value::Null,
    )
    .await;
    assert_eq!(status.1["data"]["user"]["role"], "user");
    assert_eq!(
        call(
            &fixture,
            "POST",
            "/api/admin/auth/logout",
            &current,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&fixture, "GET", "/api/user/profile", &current, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(fixture.auth.audit_count(), 0, "用户业务不新增操作审计");
}
