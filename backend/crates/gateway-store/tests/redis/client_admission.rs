use std::time::Duration;

use chrono::{DateTime, Utc};
use gateway_store::redis::{
    ClientAdmissionDecision, ClientAdmissionLimits, ClientAdmissionRecentRequest,
    ClientAdmissionRejection, ClientAdmissionRepository, ClientAdmissionRequest,
    ClientAdmissionRestore, ClientAdmissionRestoreResult, ClientAdmissionRunningRequest,
    RedisClientAdmissionRepository,
};
use redis::aio::ConnectionManager;
use uuid::Uuid;

#[test]
fn client_admission_rejects_zero_ttl() {
    let request = admission_request("request-1", "key-1", Duration::ZERO);
    assert!(request.validate().is_err());
}

#[tokio::test]
async fn request_counts_follow_shared_admission_without_mutating_expired_members() {
    let Some((repo, mut connection, namespace)) = repository().await else {
        return;
    };
    let first = admission_request("usage-first", "usage-key-a", Duration::from_secs(30));
    let mut second = admission_request("usage-second", "usage-key-b", Duration::from_secs(30));
    second.user_id = first.user_id.clone();
    repo.admit_client_request(&first).await.unwrap();
    repo.admit_client_request(&first).await.unwrap();
    repo.admit_client_request(&second).await.unwrap();
    let user_ids = vec![first.user_id.clone(), "another-user".into()];
    let key_ids = vec!["usage-key-a".into(), "usage-key-b".into(), "empty".into()];
    assert_eq!(
        repo.request_counts(&user_ids, true).await.unwrap(),
        vec![(2, 2), (0, 0)]
    );
    assert_eq!(
        repo.request_counts(&key_ids, false).await.unwrap(),
        vec![(1, 1), (1, 1), (0, 0)]
    );
    repo.release_client_request(
        &first.user_id,
        &first.client_api_key_ref,
        &first.model_request_id,
    )
    .await
    .unwrap();
    assert_eq!(
        repo.request_counts(&user_ids, true).await.unwrap(),
        vec![(1, 2), (0, 0)]
    );
    assert_eq!(
        repo.request_counts(&key_ids, false).await.unwrap(),
        vec![(0, 1), (1, 1), (0, 0)]
    );
    let mut rejected = admission_request("usage-rejected", "usage-key-a", Duration::from_secs(30));
    rejected.user_id = first.user_id.clone();
    rejected.user_limits.max_concurrency = 1;
    assert_eq!(
        repo.admit_client_request(&rejected).await.unwrap(),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::ConcurrencyLimited)
    );
    assert_eq!(
        repo.request_counts(&user_ids, true).await.unwrap(),
        vec![(1, 2), (0, 0)]
    );
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("{namespace}:client:*"))
        .query_async(&mut connection)
        .await
        .unwrap();
    for key in &keys {
        redis::cmd("ZADD")
            .arg(key)
            .arg(1)
            .arg("expired-sample")
            .query_async::<i64>(&mut connection)
            .await
            .unwrap();
    }
    assert_eq!(
        repo.request_counts(&user_ids, true).await.unwrap(),
        vec![(1, 2), (0, 0)]
    );
    assert_eq!(
        repo.request_counts(&key_ids, false).await.unwrap(),
        vec![(0, 1), (1, 1), (0, 0)]
    );
    for key in &keys {
        let score: Option<f64> = redis::cmd("ZSCORE")
            .arg(key)
            .arg("expired-sample")
            .query_async(&mut connection)
            .await
            .unwrap();
        assert_eq!(score, Some(1.0), "查询不得清理或修改限流状态");
    }
    assert!(
        repo.request_counts(&vec!["id".into(); 101], true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn user_concurrency_and_rpm_are_shared_and_rejections_do_not_partially_charge() {
    let Some((repository, mut connection, namespace)) = repository().await else {
        return;
    };
    let mut first = admission_request("first", "key-a", Duration::from_secs(30));
    first.user_limits = ClientAdmissionLimits {
        max_concurrency: 1,
        requests_per_minute: 2,
    };
    first.limits = ClientAdmissionLimits {
        max_concurrency: 1,
        requests_per_minute: 1,
    };
    assert_eq!(
        repository.admit_client_request(&first).await.unwrap(),
        ClientAdmissionDecision::Granted
    );
    assert_eq!(
        repository.admit_client_request(&first).await.unwrap(),
        ClientAdmissionDecision::Granted
    );
    let mut second = first.clone();
    second.model_request_id = "second".to_owned();
    second.client_api_key_ref = "key-b".to_owned();
    assert_eq!(
        repository.admit_client_request(&second).await.unwrap(),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::ConcurrencyLimited)
    );
    repository
        .release_client_request("test-owner", "key-a", "first")
        .await
        .unwrap();
    let mut key_rejected = first.clone();
    key_rejected.model_request_id = "key-rejected".to_owned();
    assert_eq!(
        repository
            .admit_client_request(&key_rejected)
            .await
            .unwrap(),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::RateLimited)
    );
    // Key RPM 拒绝后，另一个 Key 仍可消费用户的第二个名额。
    assert_eq!(
        repository.admit_client_request(&second).await.unwrap(),
        ClientAdmissionDecision::Granted
    );
    repository
        .release_client_request("test-owner", "key-b", "second")
        .await
        .unwrap();
    let mut third = second.clone();
    third.model_request_id = "third".to_owned();
    third.client_api_key_ref = "key-c".to_owned();
    assert_eq!(
        repository.admit_client_request(&third).await.unwrap(),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::RateLimited)
    );
    use sha2::{Digest as _, Sha256};
    let user_hash = hex::encode(Sha256::digest(b"user:test-owner"));
    let user_requests = format!("{namespace}:client:{{{user_hash}}}:requests");
    assert_eq!(
        zmembers(&mut connection, &user_requests).await,
        vec!["first", "second"]
    );
    let rejected_hash = hex::encode(Sha256::digest(b"key-c"));
    assert!(
        !namespace_keys(&mut connection, &namespace)
            .await
            .iter()
            .any(|key| key.contains(&rejected_hash))
    );
    delete_namespace_keys(&mut connection, &namespace).await;
}

#[tokio::test]
async fn core_recovery_restores_shared_user_rpm_across_keys() {
    use gateway_core::{
        engine::{
            ModelRequestId,
            admission::{ClientAdmissionPort as _, ClientAdmissionRecovery, RecentAdmissionFact},
        },
        policy::ClientApiKeyId,
    };
    let Some((repository, mut connection, namespace)) = repository().await else {
        return;
    };
    let now = redis_now(&mut connection).await - Duration::from_secs(1);
    for key in ["key-a", "key-b"] {
        repository
            .restore(ClientAdmissionRecovery {
                user_id: Some("test-owner".to_owned()),
                client_api_key_id: ClientApiKeyId::new(key).unwrap(),
                recent_requests: vec![RecentAdmissionFact {
                    model_request_id: ModelRequestId::new(format!("req_{key}")).unwrap(),
                    started_at: now.into(),
                }],
                running_requests: Vec::new(),
            })
            .await
            .unwrap();
    }
    let mut request = admission_request("new", "key-c", Duration::from_secs(30));
    request.user_limits.requests_per_minute = 2;
    assert_eq!(
        repository.admit_client_request(&request).await.unwrap(),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::RateLimited)
    );
    delete_namespace_keys(&mut connection, &namespace).await;
}

#[test]
fn client_admission_rejects_values_outside_redis_exact_integer_range() {
    let mut request = admission_request("request-1", "key-1", Duration::from_secs(30));
    request.limits.max_concurrency = 1_u64 << 53;
    assert!(request.validate().is_err());
}

#[test]
fn client_admission_restore_rejects_duplicate_request_ids() {
    let started_at = Utc::now();
    let recovery = ClientAdmissionRestore {
        client_api_key_ref: "key-1".to_owned(),
        recent_requests: vec![
            recent_request("request-1", started_at),
            recent_request("request-1", started_at),
        ],
        running_requests: Vec::new(),
    };
    assert!(recovery.validate().is_err());
}

#[tokio::test]
async fn restore_rebuilds_lost_cache_without_overwriting_new_admission() {
    let Some((repository, mut connection, namespace)) = repository().await else {
        return;
    };
    let key_ref = "key-cache-recovery";
    let old = admission_request("request-before-crash", key_ref, Duration::from_secs(30));
    assert_eq!(
        repository
            .admit_client_request(&old)
            .await
            .expect("admit request before cache loss"),
        ClientAdmissionDecision::Granted
    );
    delete_namespace_keys(&mut connection, &namespace).await;

    let live = admission_request("request-after-crash", key_ref, Duration::from_secs(30));
    assert_eq!(
        repository
            .admit_client_request(&live)
            .await
            .expect("admit concurrent request after cache loss"),
        ClientAdmissionDecision::Granted
    );
    let redis_now = redis_now(&mut connection).await;
    let recovery = ClientAdmissionRestore {
        client_api_key_ref: key_ref.to_owned(),
        recent_requests: vec![
            recent_request(
                "request-before-crash",
                redis_now - chrono::Duration::seconds(2),
            ),
            recent_request(
                "request-after-crash",
                redis_now - chrono::Duration::seconds(1),
            ),
        ],
        running_requests: vec![
            running_request(
                "request-before-crash",
                redis_now + chrono::Duration::seconds(180),
            ),
            running_request(
                "request-after-crash",
                redis_now + chrono::Duration::seconds(30),
            ),
        ],
    };

    let restored = repository
        .restore_client_admission(&recovery)
        .await
        .expect("merge durable facts into live admission state");
    assert_eq!(
        restored,
        ClientAdmissionRestoreResult {
            restored_recent_requests: 1,
            restored_running_requests: 1,
        }
    );
    assert_eq!(
        repository
            .restore_client_admission(&recovery)
            .await
            .expect("repeat idempotent recovery"),
        ClientAdmissionRestoreResult {
            restored_recent_requests: 0,
            restored_running_requests: 0,
        }
    );

    let keys = namespace_keys(&mut connection, &namespace).await;
    assert_eq!(
        zcard(&mut connection, key_with_suffix(&keys, ":active")).await,
        2
    );
    assert_eq!(
        zcard(&mut connection, key_with_suffix(&keys, ":requests")).await,
        2
    );
    let active_ttl = pttl(&mut connection, key_with_suffix(&keys, ":active")).await;
    assert!((230_000..=245_000).contains(&active_ttl));

    let mut probe = admission_request("request-probe", key_ref, Duration::from_secs(30));
    probe.limits.requests_per_minute = 3;
    assert_eq!(
        repository
            .admit_client_request(&probe)
            .await
            .expect("enforce restored concurrency"),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::ConcurrencyLimited)
    );
    assert!(
        repository
            .release_client_request("test-owner", key_ref, "request-before-crash")
            .await
            .expect("release restored request by durable ID")
    );
    assert_eq!(
        repository
            .admit_client_request(&probe)
            .await
            .expect("reuse released restored slot within RPM limit"),
        ClientAdmissionDecision::Granted
    );
    assert!(
        repository
            .release_client_request("test-owner", key_ref, "request-after-crash")
            .await
            .expect("release admission created during recovery")
    );
    let mut rate_probe = admission_request("request-rate-probe", key_ref, Duration::from_secs(30));
    rate_probe.limits.requests_per_minute = 3;
    assert_eq!(
        repository
            .admit_client_request(&rate_probe)
            .await
            .expect("enforce restored RPM without duplicate request facts"),
        ClientAdmissionDecision::Rejected(ClientAdmissionRejection::RateLimited)
    );

    repository
        .clear_client_admission(key_ref)
        .await
        .expect("clean client admission state");
}

#[tokio::test]
async fn restore_uses_redis_time_for_window_and_running_expiry_boundaries() {
    let Some((repository, mut connection, namespace)) = repository().await else {
        return;
    };
    let key_ref = "key-time-boundary";
    let redis_now = redis_now(&mut connection).await;
    let recovery = ClientAdmissionRestore {
        client_api_key_ref: key_ref.to_owned(),
        recent_requests: vec![
            recent_request(
                "request-at-cutoff",
                redis_now - chrono::Duration::seconds(60),
            ),
            recent_request(
                "request-inside-window",
                redis_now - chrono::Duration::seconds(59),
            ),
        ],
        running_requests: vec![
            running_request(
                "request-expired",
                redis_now - chrono::Duration::milliseconds(1),
            ),
            running_request("request-live", redis_now + chrono::Duration::seconds(180)),
        ],
    };

    assert_eq!(
        repository
            .restore_client_admission(&recovery)
            .await
            .expect("restore time-boundary facts"),
        ClientAdmissionRestoreResult {
            restored_recent_requests: 1,
            restored_running_requests: 1,
        }
    );
    let keys = namespace_keys(&mut connection, &namespace).await;
    let request_members = zmembers(&mut connection, key_with_suffix(&keys, ":requests")).await;
    assert_eq!(request_members, vec!["request-inside-window"]);
    let active_members = zmembers(&mut connection, key_with_suffix(&keys, ":active")).await;
    assert_eq!(active_members, vec!["request-live"]);
    let active_ttl = pttl(&mut connection, key_with_suffix(&keys, ":active")).await;
    assert!((230_000..=245_000).contains(&active_ttl));
    assert!(
        repository
            .release_client_request("test-owner", key_ref, "request-live")
            .await
            .expect("release live recovered request")
    );

    repository
        .clear_client_admission(key_ref)
        .await
        .expect("clean time-boundary state");
}

#[tokio::test]
async fn restore_rejects_future_window_fact_without_partial_write() {
    let Some((repository, mut connection, namespace)) = repository().await else {
        return;
    };
    let redis_now = redis_now(&mut connection).await;
    let recovery = ClientAdmissionRestore {
        client_api_key_ref: "key-future-fact".to_owned(),
        recent_requests: vec![
            recent_request("request-valid", redis_now - chrono::Duration::seconds(1)),
            recent_request("request-future", redis_now + chrono::Duration::seconds(10)),
        ],
        running_requests: Vec::new(),
    };

    assert!(
        repository
            .restore_client_admission(&recovery)
            .await
            .is_err()
    );
    assert!(namespace_keys(&mut connection, &namespace).await.is_empty());
}

fn admission_request(
    model_request_id: &str,
    client_api_key_ref: &str,
    lease_ttl: Duration,
) -> ClientAdmissionRequest {
    ClientAdmissionRequest {
        user_id: "test-owner".to_owned(),
        user_limits: ClientAdmissionLimits {
            max_concurrency: 0,
            requests_per_minute: 0,
        },
        model_request_id: model_request_id.to_owned(),
        client_api_key_ref: client_api_key_ref.to_owned(),
        lease_ttl,
        limits: ClientAdmissionLimits {
            max_concurrency: 2,
            requests_per_minute: 0,
        },
    }
}

fn recent_request(
    model_request_id: &str,
    started_at: DateTime<Utc>,
) -> ClientAdmissionRecentRequest {
    ClientAdmissionRecentRequest {
        model_request_id: model_request_id.to_owned(),
        started_at,
    }
}

fn running_request(
    model_request_id: &str,
    expires_at: DateTime<Utc>,
) -> ClientAdmissionRunningRequest {
    ClientAdmissionRunningRequest {
        model_request_id: model_request_id.to_owned(),
        expires_at,
    }
}

async fn repository() -> Option<(RedisClientAdmissionRepository, ConnectionManager, String)> {
    let redis_url = crate::support::test_env("CPR_TEST_REDIS_URL")?;
    let client = redis::Client::open(redis_url).expect("valid CPR_TEST_REDIS_URL");
    let connection = client
        .get_connection_manager()
        .await
        .expect("connect test Redis");
    let namespace = format!("gateway-store-admission-test-{}", Uuid::new_v4());
    let repository = RedisClientAdmissionRepository::new(connection.clone(), &namespace)
        .expect("valid test namespace");
    Some((repository, connection, namespace))
}

async fn redis_now(connection: &mut ConnectionManager) -> DateTime<Utc> {
    let (seconds, microseconds) = redis::cmd("TIME")
        .query_async::<(i64, i64)>(connection)
        .await
        .expect("read Redis server time");
    DateTime::from_timestamp(
        seconds,
        u32::try_from(microseconds).expect("valid microseconds") * 1_000,
    )
    .expect("Redis time is representable")
}

async fn namespace_keys(connection: &mut ConnectionManager, namespace: &str) -> Vec<String> {
    redis::cmd("KEYS")
        .arg(format!("{namespace}:*"))
        .query_async(connection)
        .await
        .expect("list isolated admission keys")
}

async fn delete_namespace_keys(connection: &mut ConnectionManager, namespace: &str) {
    let keys = namespace_keys(connection, namespace).await;
    if !keys.is_empty() {
        redis::cmd("DEL")
            .arg(keys)
            .query_async::<i64>(connection)
            .await
            .expect("delete isolated admission keys");
    }
}

fn key_with_suffix<'a>(keys: &'a [String], suffix: &str) -> &'a str {
    use sha2::{Digest as _, Sha256};
    let user_fingerprint = hex::encode(Sha256::digest(b"user:test-owner"));
    keys.iter()
        .find(|key| key.ends_with(suffix) && !key.contains(&user_fingerprint))
        .expect("admission key with expected suffix")
}

async fn zcard(connection: &mut ConnectionManager, key: &str) -> u64 {
    redis::cmd("ZCARD")
        .arg(key)
        .query_async(connection)
        .await
        .expect("read admission cardinality")
}

async fn zmembers(connection: &mut ConnectionManager, key: &str) -> Vec<String> {
    redis::cmd("ZRANGE")
        .arg(key)
        .arg(0)
        .arg(-1)
        .query_async(connection)
        .await
        .expect("read admission members")
}

async fn pttl(connection: &mut ConnectionManager, key: &str) -> i64 {
    redis::cmd("PTTL")
        .arg(key)
        .query_async(connection)
        .await
        .expect("read admission TTL")
}
