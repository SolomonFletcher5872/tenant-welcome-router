use crate::infrai_client::{ClientError, CreateUser, InfraiClient};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct OnboardingService {
    infrai: InfraiClient,
    tenants: Arc<RwLock<HashMap<String, TenantRecord>>>,
}

#[derive(Debug, Deserialize)]
pub struct OnboardTenant {
    pub onboarding_id: String,
    pub tenant_name: String,
    pub admin_name: String,
    pub email: String,
    pub phone: String,
    pub password: String,
    pub signed_up_with: Channel,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Channel { Email, Sms }

#[derive(Clone, Debug, Serialize)]
pub struct TenantRecord {
    pub tenant_id: String,
    pub user_id: String,
    pub tenant_name: String,
    pub lifecycle: Lifecycle,
    pub welcome_channel: Channel,
    pub message_id: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle { Active }

#[derive(Debug, Error)]
pub enum OnboardingError {
    #[error("both welcome channels are suppressed")]
    NoDeliverableChannel,
    #[error(transparent)]
    Infrai(#[from] ClientError),
}

impl OnboardingService {
    pub fn new(infrai: InfraiClient) -> Self {
        Self { infrai, tenants: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn onboard(&self, input: OnboardTenant) -> Result<TenantRecord, OnboardingError> {
        if let Some(existing) = self.tenants.read().await.get(&input.onboarding_id) {
            return Ok(existing.clone());
        }

        let channel = choose_channel(
            input.signed_up_with,
            self.infrai.email_suppressed(&input.email).await?,
            self.infrai.sms_suppressed(&input.phone).await?,
        )?;
        let created = self.infrai.create_user(&CreateUser {
            email: &input.email,
            password: &input.password,
            name: &input.admin_name,
            metadata: json!({"tenant_name": input.tenant_name}),
            mode: "X",
            idempotency_key: input.onboarding_id.clone(),
        }).await?;
        let send_key = format!("{}-welcome", input.onboarding_id);
        let message_id = match channel {
            Channel::Email => self.infrai.send_email(&input.email, &input.admin_name, &send_key).await?,
            Channel::Sms => self.infrai.send_sms(&input.phone, &input.admin_name, &send_key).await?,
        };
        let record = TenantRecord {
            tenant_id: input.onboarding_id.clone(),
            user_id: created.user_id,
            tenant_name: input.tenant_name,
            lifecycle: Lifecycle::Active,
            welcome_channel: channel,
            message_id,
        };
        self.tenants.write().await.insert(input.onboarding_id, record.clone());
        Ok(record)
    }

    pub async fn tenant(&self, tenant_id: &str) -> Option<TenantRecord> {
        self.tenants.read().await.get(tenant_id).cloned()
    }
}

pub fn choose_channel(preferred: Channel, email_suppressed: bool, sms_suppressed: bool) -> Result<Channel, OnboardingError> {
    match (preferred, email_suppressed, sms_suppressed) {
        (Channel::Email, false, _) => Ok(Channel::Email),
        (Channel::Sms, _, false) => Ok(Channel::Sms),
        (Channel::Email, true, false) => Ok(Channel::Sms),
        (Channel::Sms, false, true) => Ok(Channel::Email),
        (_, true, true) => Err(OnboardingError::NoDeliverableChannel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_sms_when_signup_email_is_suppressed() {
        let selected = choose_channel(Channel::Email, true, false).unwrap();
        assert_eq!(selected, Channel::Sms);
    }

    #[test]
    fn rejects_when_neither_channel_can_receive_the_welcome() {
        assert!(matches!(choose_channel(Channel::Sms, true, true), Err(OnboardingError::NoDeliverableChannel)));
    }
}

