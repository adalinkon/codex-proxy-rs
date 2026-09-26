use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request},
};
use gateway_admin::{
    model::system::{
        SystemOperationAccepted, SystemUpdateDetail, SystemUpdateStatus, SystemVersion,
    },
    ports::system::{SystemOperationError, SystemOperations, SystemUpdateEventStream},
};
use gateway_core::{
    engine::execution::{ClientAuthenticationError, ClientKeyVerifier},
    policy::ClientApiKeyId,
};
use serde_json::Value;

pub(crate) const RAW_KEY: &str = "cpr_live_candidate_must_not_leak";

pub(crate) async fn key_fixture() -> crate::admin::AdminTestFixture {
    crate::admin::AdminTestFixture::with_key_verifier(
        Arc::new(AcceptingVerifier {
            key_id: ClientApiKeyId::new("key-42").expect("key ID"),
        }),
        Arc::new(VersionSystem),
    )
    .await
}

pub(crate) fn empty_request(method: Method, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("client empty request")
}

pub(crate) async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read client response body");
    serde_json::from_slice(&body).expect("parse client response JSON")
}

struct VersionSystem;

#[async_trait]
impl SystemOperations for VersionSystem {
    async fn version(&self) -> Result<SystemVersion, SystemOperationError> {
        Ok(SystemVersion {
            version: "3.7.0".to_owned(),
            git_sha: "internal-revision".to_owned(),
            build_time: "internal-build-time".to_owned(),
            deployment_mode: "binary".to_owned(),
            update_channel: "release".to_owned(),

            latest_version: "3.8.0".to_owned(),
            has_update: true,
            update_cached: true,
            update_warning: Some("internal-update-warning".to_owned()),
        })
    }

    async fn update_detail(
        &self,
        _: bool,
        _: Option<gateway_admin::model::system::SystemUpdateChannel>,
    ) -> Result<SystemUpdateDetail, SystemOperationError> {
        unreachable!("client route must not request update details")
    }

    fn update_events(&self) -> SystemUpdateEventStream {
        unreachable!("client route must not subscribe to updates")
    }

    async fn perform_update(
        &self,
        _: Option<String>,
        _: Option<gateway_admin::model::system::SystemUpdateChannel>,
        _: std::sync::Arc<dyn gateway_admin::ports::system::SystemUpdatePreflight>,
    ) -> Result<SystemOperationAccepted, SystemOperationError> {
        unreachable!("client route must not perform updates")
    }

    async fn update_status(&self) -> Result<SystemUpdateStatus, SystemOperationError> {
        unreachable!("client route must not request update status")
    }

    async fn rollback(
        &self,
        _: std::sync::Arc<dyn gateway_admin::ports::system::SystemUpdatePreflight>,
    ) -> Result<SystemOperationAccepted, SystemOperationError> {
        unreachable!("client route must not roll back")
    }

    async fn restart(&self) -> Result<SystemOperationAccepted, SystemOperationError> {
        unreachable!("client route must not restart")
    }
}

struct AcceptingVerifier {
    key_id: ClientApiKeyId,
}

impl ClientKeyVerifier for AcceptingVerifier {
    fn verify_client_key(
        &self,
        candidate: &str,
    ) -> Result<ClientApiKeyId, ClientAuthenticationError> {
        if candidate == RAW_KEY {
            Ok(self.key_id.clone())
        } else {
            Err(ClientAuthenticationError::InvalidKey)
        }
    }
}
