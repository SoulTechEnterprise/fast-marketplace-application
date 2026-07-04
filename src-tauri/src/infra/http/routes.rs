use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};

use crate::infra::http::{
    handlers::{
        add_property::add_property, add_vehicle::add_vehicle, get_marketplace::get_marketplace,
        healthz::healthz, renew_listings::renew_listings,
        signin_marketplace::signin_marketplace,
        signout_marketplace::signout_marketplace, status::get_status,
    },
    setup::AppState,
};

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/property", post(add_property))
        .route("/vehicle", post(add_vehicle))
        .route("/sign-in", post(signin_marketplace))
        .route("/sign-out", post(signout_marketplace))
        .route("/marketplace", post(get_marketplace))
        .route("/renew-listings", post(renew_listings))
        .route("/healthz", get(healthz))
        .route("/status", get(get_status))
        // Imagens tratadas chegam como base64 no corpo (property/vehicle), o que
        // estoura o limite padrão do axum (2 MB). 256 MB cobre lotes grandes.
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .with_state(state)
}
