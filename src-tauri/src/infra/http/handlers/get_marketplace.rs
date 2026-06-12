use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::infra::{
    http::{
        dtos::marketplace::{MarketplaceUseCaseRequest, MarketplaceUseCaseResponse},
        setup::AppState,
    },
    logger,
    status::AppStatus,
};

pub async fn get_marketplace(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MarketplaceUseCaseRequest>,
) -> Result<Json<MarketplaceUseCaseResponse>, (StatusCode, String)> {
    let MarketplaceUseCaseRequest { client_id } = payload;

    state.status.set(AppStatus::verificando());

    let result = state
        .get_marketplace_usecase
        .handle(client_id)
        .await
        .map_err(|e| {
            state.status.set(AppStatus::standby());
            let msg = format!("Falha ao verificar login no Facebook: {}", e);
            logger::error(&msg);
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        })?;

    state.status.set(AppStatus::standby());

    Ok(Json(MarketplaceUseCaseResponse { status: result }))
}
