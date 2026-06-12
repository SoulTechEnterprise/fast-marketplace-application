use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Tipos de status
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum StatusKind {
    #[serde(rename = "standby")]
    Standby,
    #[serde(rename = "verificando")]
    Verificando,
    #[serde(rename = "entrando")]
    Entrando,
    #[serde(rename = "publicando")]
    Publicando,
    #[serde(rename = "publicado")]
    Publicado,
    #[serde(rename = "erro")]
    Erro,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppStatus {
    pub kind: StatusKind,
    pub message: String,
}

impl AppStatus {
    pub fn standby() -> Self {
        Self {
            kind: StatusKind::Standby,
            message: "Aguardando".to_string(),
        }
    }

    pub fn verificando() -> Self {
        Self {
            kind: StatusKind::Verificando,
            message: "Verificando conta...".to_string(),
        }
    }

    pub fn entrando() -> Self {
        Self {
            kind: StatusKind::Entrando,
            message: "Aguardando login no Facebook...".to_string(),
        }
    }

    pub fn publicando(msg: &str) -> Self {
        Self {
            kind: StatusKind::Publicando,
            message: msg.to_string(),
        }
    }

    pub fn publicado(msg: &str) -> Self {
        Self {
            kind: StatusKind::Publicado,
            message: msg.to_string(),
        }
    }

    pub fn erro(msg: &str) -> Self {
        Self {
            kind: StatusKind::Erro,
            message: msg.to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Gerenciador de status
// ─────────────────────────────────────────────────────────────────────────────

/// Handle compartilhado do status da aplicação.
/// Pode ser clonado livremente e passado para qualquer handler.
///
/// O contador de geração evita uma condição de corrida: um reset agendado
/// por `set_with_reset` só é aplicado se nenhum status mais novo tiver sido
/// definido nesse meio tempo (senão ele apagaria, por exemplo, um
/// "Publicando..." recém-iniciado).
#[derive(Clone)]
pub struct StatusHandle(Arc<RwLock<(AppStatus, u64)>>);

impl StatusHandle {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new((AppStatus::standby(), 0))))
    }

    /// Define o status e retorna a geração atribuída.
    fn set_internal(&self, status: AppStatus) -> u64 {
        if let Ok(mut s) = self.0.write() {
            s.1 = s.1.wrapping_add(1);
            s.0 = status;
            s.1
        } else {
            0
        }
    }

    pub fn set(&self, status: AppStatus) {
        self.set_internal(status);
    }

    pub fn get(&self) -> AppStatus {
        self.0
            .read()
            .map(|s| s.0.clone())
            .unwrap_or_else(|_| AppStatus::standby())
    }

    /// Define o status e agenda um reset automático para Standby após `secs`
    /// segundos — mas só se nenhum outro status tiver sido definido depois.
    pub fn set_with_reset(&self, status: AppStatus, secs: u64) {
        let generation = self.set_internal(status);
        let handle = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(secs)).await;
            if let Ok(mut s) = handle.0.write() {
                if s.1 == generation {
                    s.1 = s.1.wrapping_add(1);
                    s.0 = AppStatus::standby();
                }
            }
        });
    }
}
