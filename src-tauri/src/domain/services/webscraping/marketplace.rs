use async_trait::async_trait;

use crate::domain::{
    entities::{property::Property, vehicle::Vehicle},
    services::error::DomainError,
};

#[async_trait]
pub trait WebscrapingMarketplaceService: Send + Sync {
    async fn add_property(&self, entity: Property, client_id: String) -> Result<(), DomainError>;
    async fn add_vehicle(&self, entity: Vehicle, client_id: String) -> Result<(), DomainError>;

    async fn signin(&self, client_id: String) -> Result<(), DomainError>;
    async fn signout(&self, client_id: String) -> Result<(), DomainError>;
    async fn get_account(&self, client_id: String) -> Result<bool, DomainError>;

    /// Abre o painel "Renovar classificados" e renova todos os anúncios
    /// elegíveis, recarregando até não sobrar nenhum. Retorna o total renovado.
    async fn renew_listings(&self, client_id: String) -> Result<u32, DomainError>;
}
