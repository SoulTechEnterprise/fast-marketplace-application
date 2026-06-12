use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::infra::{
    http::{
        dtos::add_vehicle::{AddVehicleUseCaseRequest, AddVehicleUseCaseResponse},
        setup::AppState,
    },
    status::AppStatus,
};

pub async fn add_vehicle(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AddVehicleUseCaseRequest>,
) -> Result<Json<AddVehicleUseCaseResponse>, (StatusCode, Json<serde_json::Value>)> {
    let AddVehicleUseCaseRequest { client_id, vehicle } = payload;

    state.status.set(AppStatus::publicando("Publicando veículo no Marketplace..."));

    let result = state.vehicle_usecase.handle(client_id, vehicle).await;

    match &result {
        Ok(_) => {
            state.status.set_with_reset(AppStatus::publicado("Veículo publicado com sucesso!"), 8);
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

    Ok(Json(AddVehicleUseCaseResponse {}))
}
