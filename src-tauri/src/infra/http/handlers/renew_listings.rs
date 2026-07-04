use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::infra::{
    http::{
        dtos::marketplace::{MarketplaceUseCaseRequest, RenewListingsUseCaseResponse},
        setup::AppState,
    },
    status::AppStatus,
};

pub async fn renew_listings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MarketplaceUseCaseRequest>,
) -> Result<Json<RenewListingsUseCaseResponse>, (StatusCode, Json<serde_json::Value>)> {
    let MarketplaceUseCaseRequest { client_id } = payload;

    state
        .status
        .set(AppStatus::publicando("Renovando anúncios..."));

    match state.renew_listings_usecase.handle(client_id).await {
        Ok(renewed) => {
            state.status.set_with_reset(
                AppStatus::publicado(&format!("{} anúncio(s) renovado(s)", renewed)),
                8,
            );
            Ok(Json(RenewListingsUseCaseResponse { renewed }))
        }
        Err(e) => {
            let msg = match e {
                crate::application::error::UseCasesError::Domain(
                    crate::domain::services::error::DomainError::AutomationError(m),
                ) => m,
                other => other.to_string(),
            };
            state.status.set_with_reset(AppStatus::erro(&msg), 12);
            Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            ))
        }
    }
}
