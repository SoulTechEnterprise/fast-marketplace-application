use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::infra::{
    http::{
        dtos::marketplace::{MarketplaceUseCaseRequest, MarketplaceUseCaseResponse},
        setup::AppState,
    },
    status::AppStatus,
};

pub async fn get_marketplace(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MarketplaceUseCaseRequest>,
) -> Result<Json<MarketplaceUseCaseResponse>, StatusCode> {
    let MarketplaceUseCaseRequest { client_id } = payload;

    state.status.set(AppStatus::verificando());

    let result = state
        .get_marketplace_usecase
        .handle(client_id)
        .await
        .map_err(|_| {
            state.status.set(AppStatus::standby());
            StatusCode::NOT_FOUND
        })?;

    state.status.set(AppStatus::standby());

    Ok(Json(MarketplaceUseCaseResponse { status: result }))
}
