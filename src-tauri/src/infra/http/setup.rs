use std::sync::Arc;

use crate::{
    application::usecases::{
        add_property::AddPropertyUseCase, add_vehicle::AddVehicleUseCase,
        get_marketplace::GetMarketplaceUseCase, renew_listings::RenewListingsUseCase,
        signin_marketplace::SignInMarketplaceUseCase,
        signout_marketplace::SignOutMarketplaceUseCase,
    },
    infra::{
        repositories::image::ImageRepositoryImpl,
        services::webscraping::marketplace::FacebookMarketplaceService,
        status::StatusHandle,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub status: StatusHandle,
    pub property_usecase: Arc<AddPropertyUseCase<ImageRepositoryImpl, FacebookMarketplaceService>>,
    pub vehicle_usecase: Arc<AddVehicleUseCase<ImageRepositoryImpl, FacebookMarketplaceService>>,
    pub get_marketplace_usecase: Arc<GetMarketplaceUseCase<FacebookMarketplaceService>>,
    pub signin_marketplace_usecase: Arc<SignInMarketplaceUseCase<FacebookMarketplaceService>>,
    pub signout_marketplace_usecase: Arc<SignOutMarketplaceUseCase<FacebookMarketplaceService>>,
    pub renew_listings_usecase: Arc<RenewListingsUseCase<FacebookMarketplaceService>>,
}
