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
#[derive(Clone)]
pub struct StatusHandle(Arc<RwLock<AppStatus>>);

impl StatusHandle {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(AppStatus::standby())))
    }

    pub fn set(&self, status: AppStatus) {
        if let Ok(mut s) = self.0.write() {
            *s = status;
        }
    }

    pub fn get(&self) -> AppStatus {
        self.0.read().map(|s| s.clone()).unwrap_or_else(|_| AppStatus::standby())
    }

    /// Define o status e agenda um reset automático para Standby após `secs` segundos.
    pub fn set_with_reset(&self, status: AppStatus, secs: u64) {
        self.set(status);
        let handle = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(secs)).await;
            handle.set(AppStatus::standby());
        });
    }
}
