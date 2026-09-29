use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use tenant_welcome_router::{
    infrai_client::{ClientError, InfraiClient},
    tenant_onboarding::{OnboardTenant, OnboardingError, OnboardingService, TenantRecord},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = OnboardingService::new(InfraiClient::from_env()?);
    let app = Router::new()
        .route("/tenants/onboard", post(onboard))
        .route("/admin/tenants/:tenant_id", get(tenant))
        .with_state(service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("tenant onboarding service listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn onboard(
    State(service): State<OnboardingService>,
    Json(input): Json<OnboardTenant>,
) -> Result<(StatusCode, Json<TenantRecord>), ServiceError> {
    let record = service.onboard(input).await?;
    Ok((StatusCode::CREATED, Json(record)))
}

async fn tenant(
    State(service): State<OnboardingService>,
    Path(tenant_id): Path<String>,
) -> Result<Json<TenantRecord>, ServiceError> {
    service.tenant(&tenant_id).await.map(Json).ok_or(ServiceError::NotFound)
}

enum ServiceError { Onboarding(OnboardingError), NotFound }

impl From<OnboardingError> for ServiceError {
    fn from(value: OnboardingError) -> Self { Self::Onboarding(value) }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "tenant not found".to_owned()),
            Self::Onboarding(OnboardingError::NoDeliverableChannel) => (StatusCode::UNPROCESSABLE_ENTITY, "no deliverable welcome channel".to_owned()),
            Self::Onboarding(OnboardingError::Infrai(ClientError::Api { status, message, .. })) if status.is_client_error() => {
                (StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_REQUEST), message)
            }
            Self::Onboarding(error) => (StatusCode::BAD_GATEWAY, error.to_string()),
        };
        (status, Json(json!({"error": message}))).into_response()
    }
}
