use std::sync::Arc;

use axum::{Json, extract::State};

use crate::infra::{http::setup::AppState, status::AppStatus};

pub async fn get_status(State(state): State<Arc<AppState>>) -> Json<AppStatus> {
    Json(state.status.get())
}
