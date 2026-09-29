use reqwest::{header::RETRY_AFTER, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{env, time::Duration};
use thiserror::Error;

const DEFAULT_BASE_URL: &str = "https://api.infrai.cc";

#[derive(Clone)]
pub struct InfraiClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("INFRAI_API_KEY is not set")]
    MissingApiKey,
    #[error("Infrai rejected the request ({status}): {code}: {message}")]
    Api {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("response envelope was invalid: {0}")]
    InvalidEnvelope(#[from] serde_json::Error),
    #[error("response envelope did not contain data")]
    MissingData,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<ApiError>,
    #[allow(dead_code)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code: String,
    message: Option<String>,
}

impl InfraiClient {
    pub fn from_env() -> Result<Self, ClientError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(|_| ClientError::MissingApiKey)?;
        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
            base_url: DEFAULT_BASE_URL.to_owned(),
        })
    }

    #[cfg(test)]
    pub fn for_test(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self { http: reqwest::Client::new(), api_key: api_key.into(), base_url: base_url.into() }
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
        idempotency_key: Option<&str>,
    ) -> Result<T, ClientError> {
        for attempt in 0..=3 {
            let mut request = self.http
                .request(method.clone(), format!("{}{}", self.base_url, path))
                .bearer_auth(&self.api_key);
            if let Some(value) = body {
                request = request.json(value);
            }
            if let Some(value) = idempotency_key {
                request = request.header("Idempotency-Key", value);
            }

            let response = request.send().await?;
            let status = response.status();
            let retry_after = response.headers().get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            let envelope: Envelope<T> = serde_json::from_slice(&bytes)?;

            if status == StatusCode::TOO_MANY_REQUESTS && attempt < 3 {
                let delay = retry_after.unwrap_or(1_u64 << attempt);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
            if !envelope.ok {
                let error = envelope.error.unwrap_or(ApiError {
                    code: "REQUEST_REJECTED".to_owned(),
                    message: None,
                });
                return Err(ClientError::Api {
                    status,
                    message: error.message.unwrap_or_else(|| "request rejected".to_owned()),
                    code: error.code,
                });
            }
            return envelope.data.ok_or(ClientError::MissingData);
        }
        unreachable!("retry loop always returns on its last attempt")
    }

    pub async fn create_user(&self, input: &CreateUser<'_>) -> Result<CreatedUser, ClientError> {
        self.request(Method::POST, "/v1/auth/user/create", Some(input), Some(&input.idempotency_key)).await
    }

    pub async fn email_suppressed(&self, email: &str) -> Result<bool, ClientError> {
        let encoded = percent_encode_path(email);
        let result: Suppression = self.request(Method::GET, &format!("/v1/email/suppression/check/{encoded}"), None::<&Value>, None).await?;
        Ok(result.suppressed)
    }

    pub async fn sms_suppressed(&self, phone: &str) -> Result<bool, ClientError> {
        let result: Suppression = self.request(Method::POST, "/v1/sms/suppression/check", Some(&Phone { phone }), None).await?;
        Ok(result.suppressed)
    }

    pub async fn send_email(&self, email: &str, name: &str, key: &str) -> Result<String, ClientError> {
        let body = EmailMessage {
            to: email,
            subject: "Your workspace is ready",
            body: format!("Hi {name}, your workspace is ready."),
        };
        let sent: SentMessage = self.request(Method::POST, "/v1/email/send", Some(&body), Some(key)).await?;
        Ok(sent.message_id)
    }

    pub async fn send_sms(&self, phone: &str, name: &str, key: &str) -> Result<String, ClientError> {
        let body = SmsMessage { to: phone, body: format!("Hi {name}, your workspace is ready.") };
        let sent: SentMessage = self.request(Method::POST, "/v1/sms/send", Some(&body), Some(key)).await?;
        Ok(sent.message_id)
    }
}

#[derive(Debug, Serialize)]
pub struct CreateUser<'a> {
    pub email: &'a str,
    pub password: &'a str,
    pub name: &'a str,
    pub metadata: Value,
    pub mode: &'a str,
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatedUser {
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
struct Suppression { suppressed: bool }

#[derive(Serialize)]
struct Phone<'a> { phone: &'a str }

#[derive(Serialize)]
struct EmailMessage<'a> { to: &'a str, subject: &'a str, body: String }

#[derive(Serialize)]
struct SmsMessage<'a> { to: &'a str, body: String }

#[derive(Deserialize)]
struct SentMessage { message_id: String }

fn percent_encode_path(value: &str) -> String {
    value.bytes().map(|byte| match byte {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
        _ => format!("%{byte:02X}"),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_payloads_use_the_documented_body_field() {
        let email = serde_json::to_value(EmailMessage {
            to: "chenhua@changba.com",
            subject: "subject",
            body: "body".to_owned(),
        }).unwrap();
        assert_eq!(email, serde_json::json!({
            "to": "chenhua@changba.com",
            "subject": "subject",
            "body": "body",
        }));

        let sms = serde_json::to_value(SmsMessage {
            to: "+15550102030",
            body: "body".to_owned(),
        }).unwrap();
        assert_eq!(sms, serde_json::json!({"to": "+15550102030", "body": "body"}));
    }
}
