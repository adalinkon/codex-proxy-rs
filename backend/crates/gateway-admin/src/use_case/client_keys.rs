//! Client API Key 管理用例。

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_core::policy::ClientApiKeyId;
use gateway_core::runtime::SnapshotControl;
use rand_core::{OsRng, RngCore as _};
use uuid::Uuid;

use crate::{
    model::{
        AdminError, MutationContext,
        client_keys::{
            ClientKeyCursorValue, ClientKeyListQuery, ClientKeyMutation, ClientKeyPage,
            ClientKeySecret, ClientKeySortField, CreateClientKey, CreatedClientKey,
            DeleteClientKey, NewClientKey, SetClientKeyEnabled, UpdateClientKey,
        },
    },
    ports::store::{AdminStoreError, AdminStoreErrorKind, ClientKeyStore},
};

use super::{map_store_error, publish_committed};

/// API 消费的 Client Key 管理服务。
#[async_trait]
pub trait ClientKeyService: Send + Sync {
    async fn update_owned_identity(
        &self,
        user_id: &str,
        id: ClientApiKeyId,
        name: String,
        label: Option<String>,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn reveal_owned(
        &self,
        user_id: &str,
        id: &ClientApiKeyId,
    ) -> Result<ClientKeySecret, AdminError>;
    async fn create_owned(
        &self,
        user_id: &str,
        command: CreateClientKey,
    ) -> Result<CreatedClientKey, AdminError>;
    async fn mutate_owned(
        &self,
        user_id: &str,
        id: ClientApiKeyId,
        enabled: Option<bool>,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn list(&self, query: ClientKeyListQuery) -> Result<ClientKeyPage, AdminError>;
    async fn reveal(&self, id: &ClientApiKeyId) -> Result<ClientKeySecret, AdminError>;
    async fn create(
        &self,
        context: &MutationContext,
        command: CreateClientKey,
    ) -> Result<CreatedClientKey, AdminError>;
    async fn update(
        &self,
        context: &MutationContext,
        command: UpdateClientKey,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn set_enabled(
        &self,
        context: &MutationContext,
        command: SetClientKeyEnabled,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn delete(
        &self,
        context: &MutationContext,
        command: DeleteClientKey,
    ) -> Result<ClientKeyMutation, AdminError>;
}

pub(crate) struct DefaultClientKeyService {
    store: Arc<dyn ClientKeyStore>,
    snapshot: Arc<dyn SnapshotControl>,
}

impl DefaultClientKeyService {
    #[must_use]
    pub(crate) fn new(store: Arc<dyn ClientKeyStore>, snapshot: Arc<dyn SnapshotControl>) -> Self {
        Self { store, snapshot }
    }
}

#[async_trait]
impl ClientKeyService for DefaultClientKeyService {
    async fn update_owned_identity(
        &self,
        user_id: &str,
        id: ClientApiKeyId,
        name: String,
        label: Option<String>,
    ) -> Result<ClientKeyMutation, AdminError> {
        if name.trim().is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || label
                .as_ref()
                .is_some_and(|value| value.len() > 128 || value.chars().any(char::is_control))
        {
            return Err(AdminError::invalid("Key 名称或标签不合法"));
        }
        let revision = self
            .store
            .mutate_owned_key(
                user_id,
                crate::model::client_keys::OwnedKeyMutation::UpdateIdentity {
                    id: id.clone(),
                    name: name.trim().to_owned(),
                    label: label
                        .map(|value| value.trim().to_owned())
                        .filter(|value| !value.is_empty()),
                },
            )
            .await
            .map_err(|error| map_store_error(error, "client key"))?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        Ok(ClientKeyMutation {
            config_revision: revision,
            record: None,
            id,
        })
    }
    async fn reveal_owned(
        &self,
        user_id: &str,
        id: &ClientApiKeyId,
    ) -> Result<ClientKeySecret, AdminError> {
        let secret = self.reveal(id).await?;
        if secret.record.user_id != user_id {
            return Err(AdminError::not_found("Key 不存在"));
        }
        Ok(secret)
    }

    async fn create_owned(
        &self,
        user_id: &str,
        command: CreateClientKey,
    ) -> Result<CreatedClientKey, AdminError> {
        let id = ClientApiKeyId::new(format!("key_{}", Uuid::now_v7().simple()))
            .map_err(|_| AdminError::internal("Key ID 创建失败"))?;
        let plaintext = create_plaintext(command.custom_key);
        let revision = self
            .store
            .mutate_owned_key(
                user_id,
                crate::model::client_keys::OwnedKeyMutation::Create(NewClientKey {
                    id: id.clone(),
                    user_id: Some(user_id.to_owned()),
                    name: command.name,
                    label: command.label,
                    group_ids: command.group_ids,
                    limits: command.limits,
                    budget: command.budget,
                    plaintext,
                }),
            )
            .await
            .map_err(map_client_key_write_error)?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        Ok(CreatedClientKey {
            config_revision: revision,
            secret: self.reveal_owned(user_id, &id).await?,
        })
    }

    async fn mutate_owned(
        &self,
        user_id: &str,
        id: ClientApiKeyId,
        enabled: Option<bool>,
    ) -> Result<ClientKeyMutation, AdminError> {
        let mutation = match enabled {
            Some(enabled) => {
                crate::model::client_keys::OwnedKeyMutation::SetEnabled(SetClientKeyEnabled {
                    id: id.clone(),
                    enabled,
                })
            }
            None => crate::model::client_keys::OwnedKeyMutation::Delete(DeleteClientKey {
                id: id.clone(),
            }),
        };
        let revision = self
            .store
            .mutate_owned_key(user_id, mutation)
            .await
            .map_err(|e| map_store_error(e, "client key"))?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        Ok(ClientKeyMutation {
            config_revision: revision,
            record: None,
            id,
        })
    }

    async fn list(&self, query: ClientKeyListQuery) -> Result<ClientKeyPage, AdminError> {
        validate_cursor(&query)?;
        self.store
            .list_client_keys(query)
            .await
            .map_err(|error| map_store_error(error, "client API key"))
    }

    async fn reveal(&self, id: &ClientApiKeyId) -> Result<ClientKeySecret, AdminError> {
        self.store
            .reveal_client_key(id)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?
            .ok_or_else(|| AdminError::not_found("Client API Key 不存在"))
    }

    async fn create(
        &self,
        context: &MutationContext,
        command: CreateClientKey,
    ) -> Result<CreatedClientKey, AdminError> {
        let id = ClientApiKeyId::new(format!("key_{}", Uuid::now_v7().simple()))
            .map_err(|_| AdminError::internal("创建 Client API Key ID 失败"))?;
        let plaintext = create_plaintext(command.custom_key);
        let (config_revision, record) = self
            .store
            .create_client_key(
                NewClientKey {
                    user_id: command.user_id.or_else(|| match &context.actor {
                        crate::model::MutationActor::AdminSession { admin_user_id } => {
                            Some(admin_user_id.clone())
                        }
                        _ => None,
                    }),
                    id,
                    name: command.name,
                    label: command.label,
                    group_ids: command.group_ids,
                    limits: command.limits,
                    budget: command.budget,
                    plaintext: plaintext.clone(),
                },
                context,
            )
            .await
            .map_err(map_client_key_write_error)?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(CreatedClientKey {
            config_revision,
            secret: ClientKeySecret::new(record, plaintext),
        })
    }

    async fn update(
        &self,
        context: &MutationContext,
        command: UpdateClientKey,
    ) -> Result<ClientKeyMutation, AdminError> {
        let id = command.id.clone();
        let (config_revision, record) = self
            .store
            .update_client_key(command, context)
            .await
            .map_err(map_client_key_write_error)?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: Some(record),
            id,
        })
    }

    async fn set_enabled(
        &self,
        context: &MutationContext,
        command: SetClientKeyEnabled,
    ) -> Result<ClientKeyMutation, AdminError> {
        let id = command.id.clone();
        let (config_revision, record) =
            self.store
                .set_client_key_enabled(command, context)
                .await
                .map_err(|error| map_store_error(error, "client API key"))?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: Some(record),
            id,
        })
    }

    async fn delete(
        &self,
        context: &MutationContext,
        command: DeleteClientKey,
    ) -> Result<ClientKeyMutation, AdminError> {
        let id = command.id.clone();
        let config_revision = self
            .store
            .delete_client_key(command, context)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: None,
            id,
        })
    }
}

fn create_plaintext(custom_key: Option<gateway_core::policy::PlaintextClientApiKey>) -> String {
    if let Some(key) = custom_key {
        return key.expose_for_auth().to_owned();
    }
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("sk_{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn map_client_key_write_error(error: AdminStoreError) -> AdminError {
    match error.kind() {
        AdminStoreErrorKind::DuplicateName => AdminError::conflict("名称已存在"),
        AdminStoreErrorKind::Conflict => AdminError::conflict("API Key 已存在，请使用其他密钥"),
        _ => map_store_error(error, "client API key"),
    }
}

fn validate_cursor(query: &ClientKeyListQuery) -> Result<(), AdminError> {
    let Some(cursor) = &query.cursor else {
        return Ok(());
    };
    if cursor.sort != query.sort {
        return Err(AdminError::invalid(
            "Client API Key 游标排序与查询条件不一致",
        ));
    }
    let matches = matches!(
        (cursor.sort.field, &cursor.value),
        (ClientKeySortField::Name, ClientKeyCursorValue::Name(value)) if !value.trim().is_empty()
    ) || matches!(
        (cursor.sort.field, &cursor.value),
        (
            ClientKeySortField::Enabled,
            ClientKeyCursorValue::Enabled(_)
        ) | (
            ClientKeySortField::CreatedAt,
            ClientKeyCursorValue::CreatedAt(_)
        ) | (
            ClientKeySortField::LastUsedAt,
            ClientKeyCursorValue::LastUsedAt(_)
        )
    );
    if matches {
        Ok(())
    } else {
        Err(AdminError::invalid("Client API Key 游标不合法"))
    }
}
