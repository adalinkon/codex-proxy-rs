use std::time::Duration;

use gateway_admin::{PluginManagementService, model::plugins::management::PluginManagementRequest};
use gateway_core::{
    lifecycle::CancellationToken,
    policy::ClientApiKeyId,
    task::{WorkerContribution, WorkerRunnable},
};
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::support::{
    environment::{Environment, account_grant},
    native,
};

#[tokio::test]
async fn plugin_process_manages_native_budget_through_client_key_service() {
    for granted in [false, true] {
        let Some(environment) = Environment::create().await else {
            eprintln!("SKIP: plugin integration environment absent");
            return;
        };
        let key = ClientApiKeyId::new("key_budget").unwrap();
        let plaintext = format!("sk_{}", "a".repeat(43));
        environment.client_key(key.as_str(), &plaintext).await;
        environment.seed_client_key_budget(key.as_str()).await;
        let store = environment.store.admin_ports().client_keys();
        let before = store.get_client_key(&key).await.unwrap().unwrap();
        environment.install_plugin(json!({
            "management_registration":{"routes":[{"method":"POST","path":"reset","request_content_types":[],"response_content_types":["application/json"]}]},
            "data_queries":[
                {"method":"host.keys.list","query":{"limit":10}},
                {"method":"host.keys.reset_budget","query":{"client_key_id":"key_budget","period":"weekly"}},
                {"method":"host.keys.reset_budget","query":{"client_key_id":"key_budget"}},
                {"method":"host.keys.reset_budget","query":{"client_key_id":"key_budget","period":"monthly"}},
                {"method":"host.keys.reset_budget","query":{"client_key_id":"key_budget","period":"all","instance_id":"forged"}},
                {"method":"host.keys.reset_budget","query":{"client_key_id":"missing","period":"all"}},
                {"method":"host.keys.get_budget","query":{"client_key_id":"key_budget"}},
                {"method":"host.keys.update_budget_limits","query":{"client_key_id":"key_budget","weekly_limit_usd":"25.5"}},
                {"method":"host.keys.get_budget","query":{"client_key_id":"key_budget"}},
                {"method":"host.keys.update_budget_limits","query":{"client_key_id":"key_budget"}},
                {"method":"host.keys.update_budget_limits","query":{"client_key_id":"key_budget","daily_limit_usd":"-1"}},
                {"method":"host.keys.get_budget","query":{"client_key_id":"missing"}},
                {"method":"host.keys.update_budget_limits","query":{"client_key_id":"missing","weekly_limit_usd":"2"}},
                {"method":"host.keys.update_budget_limits","query":{"client_key_id":"key_budget","max_concurrency":1}},
                {"method":"host.keys.get_budget","query":{"client_key_id":"key_budget","instance_id":"forged"}}
            ]
        }), if granted { vec![account_grant("key_budgets")] } else { vec![] }).await;
        let (runtime, core) = environment.runtime().await;
        let access = gateway_admin::initialize_plugin_client_keys(
            native::admin_registry(),
            store.clone(),
            core.snapshot_control(),
        );
        runtime.bind_client_key_ports(&access).unwrap();
        let service = PluginManagementService::new(
            runtime.clone(),
            environment.store.admin_ports().plugins(),
            core.snapshots(),
        );
        let view = service.views().await.unwrap().remove(0);
        let reply = service
            .handle(
                &view.target,
                PluginManagementRequest {
                    method: "POST".into(),
                    path: "reset".into(),
                    query: String::new(),
                    content_type: None,
                    body: vec![],
                    request_id: "budget-fixture".into(),
                },
            )
            .await
            .unwrap();
        let results: Vec<Value> = serde_json::from_slice(&reply.body).unwrap();
        let after = store.get_client_key(&key).await.unwrap().unwrap();
        let audits = environment.audit_requests("reset_budget").await;
        if granted {
            assert_eq!(
                results[0],
                json!({"keys":[{"id":"key_budget","name":"fixture key_budget","enabled":true}],"next_cursor":null})
            );
            for result in &results[1..6] {
                assert_eq!(result, &json!({"error":"permission_denied"}));
            }
            assert_eq!(after.budget.weekly_used_usd, before.budget.weekly_used_usd);
            assert_eq!(after.budget.daily_used_usd, before.budget.daily_used_usd);
            assert_eq!(
                after.budget.limits.daily_usd,
                before.budget.limits.daily_usd
            );
            assert_eq!(after.budget.limits.weekly_usd.canonical(), "25.5");
            let mut expected = json!({
                "client_key_id":"key_budget", "daily_limit_usd":"10", "weekly_limit_usd":"20",
                "daily_used_usd":"3", "weekly_used_usd":"4",
                "daily_resets_at_ms":before.budget.daily_resets_at.map(|time| chrono::DateTime::<chrono::Utc>::from(time).timestamp_millis()),
                "weekly_resets_at_ms":before.budget.weekly_resets_at.map(|time| chrono::DateTime::<chrono::Utc>::from(time).timestamp_millis()),
            });
            assert_eq!(results[6], expected);
            assert_eq!(results[7], json!({"client_key_id":"key_budget"}));
            expected["weekly_limit_usd"] = json!("25.5");
            assert_eq!(results[8], expected);
            for index in [9, 10, 13, 14] {
                assert_eq!(results[index], json!({"error":"invalid_input"}));
            }
            for index in [11, 12] {
                assert_eq!(results[index], json!({"error":"rejected"}));
            }
            assert_eq!(
                environment
                    .audit_requests("update_budget_limits")
                    .await
                    .len(),
                1
            );
            assert_eq!(after.budget.daily_resets_at, before.budget.daily_resets_at);
            assert_eq!(
                after.budget.weekly_resets_at,
                before.budget.weekly_resets_at
            );
            assert!(audits.is_empty());
        } else {
            assert!(
                results
                    .iter()
                    .all(|value| value == &json!({"error":"permission_denied"}))
            );
            assert_eq!(after.budget, before.budget);
            assert!(audits.is_empty());
        }
        assert_eq!(
            store
                .reveal_client_key(&key)
                .await
                .unwrap()
                .unwrap()
                .expose_for_response(),
            plaintext
        );
        assert!(
            !std::str::from_utf8(&reply.body)
                .unwrap()
                .contains(&plaintext)
        );
        drop(service);
        drop(access);
        environment.release_plugin_accounts(&runtime);
        runtime.shutdown().await;
        drop(core);
        drop(runtime);
        drop(store);
        environment.close().await;
    }
}

async fn seed_budget(environment: &Environment) -> ClientApiKeyId {
    let key = ClientApiKeyId::new("key_budget").unwrap();
    environment
        .client_key(key.as_str(), "sk-budget-fixture")
        .await;
    environment.seed_client_key_budget(key.as_str()).await;
    key
}

fn reset_queries() -> Value {
    // 即使拥有预算权限，重置也必须被拒绝；查询和上限更新仍正常工作。
    json!([
        {"method":"host.keys.reset_budget","query":{"client_key_id":"key_budget","period":"weekly"}},
        {"method":"host.keys.list","query":{"limit":10}},
        {"method":"host.keys.get_budget","query":{"client_key_id":"key_budget"}},
        {"method":"host.keys.update_budget_limits","query":{"client_key_id":"key_budget","daily_limit_usd":"10"}}
    ])
}

fn assert_reset_results(results: &Value) {
    assert_eq!(results[0], json!({"error":"permission_denied"}));
    assert_eq!(results[1]["keys"][0]["id"], "key_budget");
    assert_eq!(results[2]["daily_used_usd"], "3");
    assert_eq!(results[2]["weekly_used_usd"], "4");
    assert_eq!(results[3], json!({"client_key_id":"key_budget"}));
}

#[tokio::test]
async fn command_plane_rejects_key_reset_even_with_budget_permission() {
    let Some(mut environment) = Environment::create_command().await else {
        eprintln!("SKIP: plugin integration environment absent");
        return;
    };
    let key = seed_budget(&environment).await;
    environment
        .install_plugin(
            json!({
                "command_registration":{"commands":[{"name":"reset","description":"预算接口测试"}]},
                "data_queries":reset_queries()
            }),
            vec![account_grant("key_budgets")],
        )
        .await;
    let (runtime, core) = environment.command_plane().await;
    let store = environment.store.admin_ports().client_keys();
    let before = store.get_client_key(&key).await.unwrap().unwrap();
    let access = gateway_admin::initialize_plugin_client_keys(
        native::admin_registry(),
        store.clone(),
        core.snapshot_control(),
    );
    runtime.bind_client_key_ports(&access).unwrap();
    let instance = environment
        .store
        .admin_ports()
        .plugins()
        .load_instances()
        .await
        .unwrap()
        .instances
        .remove(0);
    let commands = runtime.prepare_command_line().await.unwrap();
    environment.store.start_command_line_writes().unwrap();
    let reply = commands.execute(&instance.id, "reset", &[]).await.unwrap();
    assert_eq!(reply.exit_code, 0);
    assert_reset_results(&serde_json::from_str(&reply.stdout).unwrap());
    let after = store.get_client_key(&key).await.unwrap().unwrap();
    assert_eq!(after.budget.weekly_used_usd, before.budget.weekly_used_usd);
    assert_eq!(after.budget.daily_used_usd, before.budget.daily_used_usd);
    assert!(environment.audit_requests("reset_budget").await.is_empty());

    commands.shutdown().await;
    runtime.shutdown().await;
    environment
        .store
        .shutdown_command_line_writes()
        .await
        .unwrap();
    drop(access);
    drop(core);
    drop(runtime);
    drop(store);
    environment.close().await;
}

#[tokio::test]
async fn published_maintenance_rejects_key_reset_even_with_budget_permission() {
    let Some(environment) = Environment::create().await else {
        eprintln!("SKIP: plugin integration environment absent");
        return;
    };
    let key = seed_budget(&environment).await;
    let marker = environment
        .directory
        .path()
        .join("budget-maintenance.jsonl");
    environment
        .install_plugin(
            json!({
                "maintenance_fixture":true,
                "maintenance_marker":marker,
                "data_queries":reset_queries()
            }),
            vec![account_grant("key_budgets")],
        )
        .await;
    let (runtime, core) = environment.runtime().await;
    let store = environment.store.admin_ports().client_keys();
    let before = store.get_client_key(&key).await.unwrap().unwrap();
    let access = gateway_admin::initialize_plugin_client_keys(
        native::admin_registry(),
        store.clone(),
        core.snapshot_control(),
    );
    runtime.bind_client_key_ports(&access).unwrap();
    let WorkerContribution::Registration(registration) =
        runtime.maintenance_worker(core.snapshots()).unwrap()
    else {
        panic!("maintenance registration")
    };
    let WorkerRunnable::Daemon { task, .. } = registration.runnable else {
        panic!("maintenance daemon")
    };
    let stop = CancellationToken::new();
    let cancellation = stop.clone();
    let worker = tokio::spawn(async move { task.run(cancellation).await.unwrap() });
    let result: Value = timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(content) = std::fs::read_to_string(&marker)
                && let Some(result) = content
                    .lines()
                    .find_map(|line| serde_json::from_str(line).ok())
            {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("maintenance callback completed");
    stop.cancel();
    timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_reset_results(&result["results"]);
    let after = store.get_client_key(&key).await.unwrap().unwrap();
    assert_eq!(after.budget.weekly_used_usd, before.budget.weekly_used_usd);
    assert_eq!(after.budget.daily_used_usd, before.budget.daily_used_usd);
    assert!(environment.audit_requests("reset_budget").await.is_empty());

    environment.release_plugin_accounts(&runtime);
    runtime.shutdown().await;
    drop(access);
    drop(core);
    drop(runtime);
    drop(store);
    environment.close().await;
}
