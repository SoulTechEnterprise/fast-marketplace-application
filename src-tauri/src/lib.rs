pub mod application;
pub mod domain;
pub mod infra;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    http::{HeaderValue, Method},
    serve,
};
use tower_http::cors::CorsLayer;

use crate::{
    application::usecases::{
        add_property::AddPropertyUseCase, add_vehicle::AddVehicleUseCase,
        get_marketplace::GetMarketplaceUseCase, signin_marketplace::SignInMarketplaceUseCase,
        signout_marketplace::SignOutMarketplaceUseCase,
    },
    infra::{
        http::{routes::routes, setup::AppState},
        logger,
        repositories::image::ImageRepositoryImpl,
        services::webscraping::marketplace::FacebookMarketplaceService,
        status::StatusHandle,
    },
};

// ─── Configuração do servidor ────────────────────────────────────────────────

const SERVER_PORT: u16 = 15137;
const SERVER_HOST: &str = "127.0.0.1";

/// Origens permitidas no CORS.
/// Inclui as origens do Tauri (custom protocol) e do dev server.
const ALLOWED_ORIGINS: [&str; 7] = [
    "http://localhost:1420",          // Tauri dev server
    "http://localhost:4000",          // Next.js dev (se usado externamente)
    "http://tauri.localhost",         // Tauri produção - Windows / Linux
    "tauri://localhost",              // Tauri produção - macOS
    "https://soultech.agency",
    "https://fast-marketplace-dev-frontend.soultech.agency",
    "https://fast-marketplace-frontend.soultech.agency",
];

// ─── Servidor HTTP (Axum) ────────────────────────────────────────────────────

/// Verifica se já existe outra instância desta aplicação respondendo na porta.
/// Evita que uma segunda instância fique rodando sem servidor (porta ocupada).
async fn another_instance_is_running() -> bool {
    let url = format!("http://{}:{}/healthz", SERVER_HOST, SERVER_PORT);
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => matches!(client.get(&url).send().await, Ok(r) if r.status().is_success()),
        Err(_) => false,
    }
}

async fn start_http_server() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    eprintln!("[HTTP] Inicializando dependências...");

    // ── Inicializar dependências ────────────────────────────────────────
    let image_repository = Arc::new(ImageRepositoryImpl::new());
    let webscraping_service = Arc::new(FacebookMarketplaceService::new());

    let property_usecase = Arc::new(AddPropertyUseCase::new(
        image_repository.clone(),
        webscraping_service.clone(),
    ));
    let vehicle_usecase = Arc::new(AddVehicleUseCase::new(
        image_repository,
        webscraping_service.clone(),
    ));
    let signin_usecase = Arc::new(SignInMarketplaceUseCase::new(webscraping_service.clone()));
    let signout_usecase = Arc::new(SignOutMarketplaceUseCase::new(webscraping_service.clone()));
    let get_marketplace_usecase = Arc::new(GetMarketplaceUseCase::new(webscraping_service));

    let status = StatusHandle::new();

    let state = Arc::new(AppState {
        status,
        property_usecase,
        vehicle_usecase,
        signin_marketplace_usecase: signin_usecase,
        signout_marketplace_usecase: signout_usecase,
        get_marketplace_usecase,
    });

    // ── Configurar CORS ─────────────────────────────────────────────────
    let allowed_origins: Vec<HeaderValue> = ALLOWED_ORIGINS
        .iter()
        .filter_map(|o| o.parse::<HeaderValue>().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    let app_router = routes(state).layer(cors);

    // ── Iniciar TCP listener ────────────────────────────────────────────
    let addr: SocketAddr = format!("{}:{}", SERVER_HOST, SERVER_PORT).parse()?;

    eprintln!("[HTTP] Criando socket em {addr}...");

    // Tenta abrir a porta com algumas tentativas: em máquinas lentas, ou após
    // um restart rápido, a porta pode ainda estar em TIME_WAIT ou ocupada por
    // uma instância anterior que está encerrando.
    let mut listener = None;
    let mut last_err: Option<std::io::Error> = None;

    for attempt in 1..=10u32 {
        let bind_result = (|| -> std::io::Result<tokio::net::TcpListener> {
            let socket = tokio::net::TcpSocket::new_v4()?;
            // SO_REUSEADDR apenas em Unix: no Windows ele permite que dois
            // processos façam bind na mesma porta simultaneamente (port hijack).
            #[cfg(not(target_os = "windows"))]
            let _ = socket.set_reuseaddr(true);
            socket.bind(addr)?;
            socket.listen(1024)
        })();

        match bind_result {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(e) => {
                logger::warn(&format!(
                    "Tentativa {}/10 de abrir a porta {} falhou: {}",
                    attempt, SERVER_PORT, e
                ));
                last_err = Some(e);

                // Se outra instância do app já estiver atendendo, encerra esta
                // tentativa sem erro fatal — a outra instância cuida de tudo.
                if another_instance_is_running().await {
                    logger::warn(
                        "Outra instância do aplicativo já está rodando nesta porta. \
                        Este processo não iniciará um segundo servidor.",
                    );
                    return Ok(());
                }

                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }

    let listener = match listener {
        Some(l) => l,
        None => {
            return Err(Box::new(last_err.unwrap_or_else(|| {
                std::io::Error::other("Falha desconhecida ao abrir a porta")
            })));
        }
    };

    logger::info(&format!(
        "Servidor HTTP rodando em http://{}:{}",
        SERVER_HOST, SERVER_PORT
    ));
    logger::separator();

    eprintln!("[HTTP] Servidor pronto em http://{SERVER_HOST}:{SERVER_PORT}");

    serve(listener, app_router).await?;

    Ok(())
}

// ─── Entry point do Tauri ────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            logger::print_banner(env!("CARGO_PKG_VERSION"), &SERVER_PORT.to_string());

            // Roda o servidor Axum numa thread dedicada com seu próprio runtime
            // Tokio — mais confiável do que depender do runtime interno do Tauri.
            std::thread::spawn(|| {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("axum-server")
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        logger::error(&format!(
                            "Falha ao criar runtime Tokio para o servidor HTTP: {e}"
                        ));
                        return;
                    }
                };

                // Se o servidor cair (erro de bind, pânico interno, etc.),
                // tenta reiniciá-lo em vez de deixar o app rodando sem backend.
                loop {
                    let result = rt.block_on(async {
                        // Captura pânicos dentro do servidor para não matar a thread.
                        match tokio::spawn(start_http_server()).await {
                            Ok(r) => r,
                            Err(join_err) => Err(format!("Pânico no servidor HTTP: {join_err}").into()),
                        }
                    });

                    match result {
                        Ok(()) => {
                            // Encerramento limpo (ex.: outra instância já roda). Não reinicia.
                            break;
                        }
                        Err(e) => {
                            logger::error(&format!(
                                "Servidor HTTP encerrou com erro: {e}. Reiniciando em 5s..."
                            ));
                            std::thread::sleep(std::time::Duration::from_secs(5));
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("Erro ao iniciar o aplicativo");
}
