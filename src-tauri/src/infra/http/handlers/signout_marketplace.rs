use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::infra::{
    http::{dtos::marketplace::MarketplaceUseCaseRequest, setup::AppState},
    status::AppStatus,
};

pub async fn signout_marketplace(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MarketplaceUseCaseRequest>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let MarketplaceUseCaseRequest { client_id } = payload;

    state.status.set(AppStatus::verificando());

    let result = state.signout_marketplace_usecase.handle(client_id).await;

    match &result {
        Ok(_) => state.status.set(AppStatus::standby()),
        Err(e) => {
            let msg = match e {
                crate::application::error::UseCasesError::Domain(
                    crate::domain::services::error::DomainError::AutomationError(m),
                ) => m.clone(),
                other => other.to_string(),
            };
            state.status.set_with_reset(AppStatus::erro(&msg), 12);
            return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))));
        }
    }

    Ok(StatusCode::OK)
}
