use std::sync::Arc;

use crate::{
    application::error::UseCasesError,
    domain::services::webscraping::marketplace::WebscrapingMarketplaceService,
};

pub struct RenewListingsUseCase<_WebscrapingMarketplaceService: WebscrapingMarketplaceService> {
    webscraping_marketplace_service: Arc<_WebscrapingMarketplaceService>,
}

impl<_WebscrapingMarketplaceService: WebscrapingMarketplaceService>
    RenewListingsUseCase<_WebscrapingMarketplaceService>
{
    pub fn new(webscraping_marketplace_service: Arc<_WebscrapingMarketplaceService>) -> Self {
        Self {
            webscraping_marketplace_service,
        }
    }

    /// Renova todos os anúncios elegíveis e devolve o total renovado.
    pub async fn handle(&self, client_id: String) -> Result<u32, UseCasesError> {
        let renewed = self
            .webscraping_marketplace_service
            .renew_listings(client_id)
            .await?;

        Ok(renewed)
    }
}
