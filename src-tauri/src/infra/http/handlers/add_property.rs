use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::infra::{
    http::{
        dtos::add_property::{AddPropertyUseCaseRequest, AddPropertyUseCaseResponse},
        setup::AppState,
    },
    status::AppStatus,
};

pub async fn add_property(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AddPropertyUseCaseRequest>,
) -> Result<Json<AddPropertyUseCaseResponse>, (StatusCode, Json<serde_json::Value>)> {
    let AddPropertyUseCaseRequest { client_id, property } = payload;

    state.status.set(AppStatus::publicando("Publicando imóvel no Marketplace..."));

    let result = state.property_usecase.handle(client_id, property).await;

    match &result {
        Ok(_) => {
            state.status.set_with_reset(AppStatus::publicado("Imóvel publicado com sucesso!"), 8);
        }
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

    Ok(Json(AddPropertyUseCaseResponse {}))
}
