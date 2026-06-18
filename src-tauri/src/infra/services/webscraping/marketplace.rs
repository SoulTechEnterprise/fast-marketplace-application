use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::{
    Element, Page,
    browser::{Browser, BrowserConfig},
};
use futures::StreamExt;
use std::path::PathBuf;
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::{Duration, sleep, timeout};

/// Tempo máximo para o Chrome iniciar. Em máquinas lentas (HD mecânico,
/// antivírus escaneando o binário) o launch pode demorar bastante.
const BROWSER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Tempo máximo para o download automático do Chromium (~150 MB).
const CHROMIUM_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

use crate::infra::logger;

use crate::domain::entities::models::property::category::Category as PropertyCategory;
use crate::domain::entities::models::property::model::Model as PropertyModel;

use crate::domain::entities::models::vehicle::bodystyle::BodyStyle as VehicleBodyStyle;
use crate::domain::entities::models::vehicle::category::Category as VehicleCategory;
use crate::domain::entities::models::vehicle::condition::Condition as VehicleCondition;
use crate::domain::entities::models::vehicle::fuel::Fuel as VehicleFuel;
use crate::domain::entities::models::vehicle::manufacturer::Manufacturer as VehicleManufacturer;
use crate::domain::{
    entities::{property::Property, vehicle::Vehicle},
    services::{error::DomainError, webscraping::marketplace::WebscrapingMarketplaceService},
};

// ─────────────────────────────────────────────────────────────────────────────
// Segurança
// ─────────────────────────────────────────────────────────────────────────────

/// Valida client_id para prevenir path traversal e injeção.
/// Permite apenas caracteres alfanuméricos, hífens e underscores.
fn sanitize_client_id(client_id: &str) -> Result<&str, DomainError> {
    if client_id.is_empty()
        || !client_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(DomainError::AutomationError(
            "Formato inválido de client_id".to_string(),
        ));
    }
    Ok(client_id)
}

// ─────────────────────────────────────────────────────────────────────────────
// Seletores CSS / constantes
// ─────────────────────────────────────────────────────────────────────────────

const SEL_PHOTO_INPUT: &str = "input[type='file']";
const SEL_FACEBOOK_LOGGED_IN: &str = "div[aria-label='Facebook']";

const SEL_FACEBOOK_TRUST_DEVICE: &str = "div[data-testid='save-device-button'], \
                                          button[name='save_device'], \
                                          div[aria-label='Salvar dispositivo'], \
                                          .__7n5 button";

// ─────────────────────────────────────────────────────────────────────────────
// Helpers de perfil / ambiente
// ─────────────────────────────────────────────────────────────────────────────

fn cleanup_stale_lock_files(dir: &std::path::Path) {
    // Remove apenas arquivos de lock que impedem o Chrome de iniciar.
    // NÃO matamos processos Chrome aqui — o BrowserGuard cuida do shutdown
    // gracioso, preservando cookies e dados de sessão.
    let _ = std::fs::remove_file(dir.join("SingletonLock"));
    let _ = std::fs::remove_file(dir.join("SingletonSocket"));
    let _ = std::fs::remove_file(dir.join("SingletonCookie"));
}

fn profile_dir(client_id: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(target_os = "windows"))]
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));

    let dir = base
        .join("marketplace")
        .join("chrome-profiles")
        .join(client_id);

    if let Err(e) = std::fs::create_dir_all(&dir) {
        logger::warn(&format!(
            "Falha ao criar pasta de perfil: {}. Usando diretório temporário.",
            e
        ));
        let temp_dir = std::env::temp_dir()
            .join("marketplace-chrome-profiles")
            .join(client_id);
        let _ = std::fs::create_dir_all(&temp_dir);
        cleanup_stale_lock_files(&temp_dir);
        return temp_dir;
    }

    cleanup_stale_lock_files(&dir);
    dir
}

// ─────────────────────────────────────────────────────────────────────────────
// Detecção robusta do executável Chrome/Chromium
// ─────────────────────────────────────────────────────────────────────────────

/// Cache do executável encontrado, para não repetir a busca (e principalmente
/// não repetir `where`/`which` nem o fetcher) a cada operação.
static CHROME_PATH_CACHE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Busca um executável Chrome/Chromium disponível no sistema, em ordem de prioridade:
/// 1. Chromium embutido na pasta `chrome-win` ao lado do executável
/// 2. Variável de ambiente CHROME_PATH
/// 3. Caminhos comuns por sistema operacional
/// 4. Busca no PATH via `which` / `where`
/// 5. Download automático via chromiumoxide fetcher (requer internet)
async fn find_chrome_executable() -> Option<PathBuf> {
    // ── 0. Cache (validando que o arquivo ainda existe) ────────────────────
    if let Ok(cache) = CHROME_PATH_CACHE.lock() {
        if let Some(p) = cache.as_ref() {
            if p.exists() {
                return Some(p.clone());
            }
        }
    }

    let found = find_chrome_executable_uncached().await;

    if let (Some(p), Ok(mut cache)) = (found.as_ref(), CHROME_PATH_CACHE.lock()) {
        *cache = Some(p.clone());
    }

    found
}

async fn find_chrome_executable_uncached() -> Option<PathBuf> {
    // ── 1. Chromium embutido (chrome-win/ ao lado do .exe) ─────────────────
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            #[cfg(target_os = "windows")]
            let bundled = exe_dir.join("chrome-win").join("chrome.exe");
            #[cfg(target_os = "macos")]
            let bundled = exe_dir.join("chrome-win").join("Google Chrome.app")
                .join("Contents").join("MacOS").join("Google Chrome");
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let bundled = exe_dir.join("chrome-win").join("chrome");

            if bundled.exists() {
                logger::info(&format!("Chromium embutido encontrado: {}", bundled.display()));
                return Some(bundled);
            }
        }
    }

    // ── 2. Variável de ambiente CHROME_PATH ─────────────────────────────────
    if let Ok(env_path) = std::env::var("CHROME_PATH") {
        let p = PathBuf::from(&env_path);
        if p.exists() {
            logger::info(&format!("Chrome via CHROME_PATH: {}", p.display()));
            return Some(p);
        }
    }

    // ── 3. Caminhos comuns por sistema operacional ──────────────────────────
    #[cfg(target_os = "windows")]
    let system_candidates: Vec<PathBuf> = {
        let mut v = vec![
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files\Chromium\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Chromium\Application\chrome.exe"),
        ];
        // %LOCALAPPDATA%\Google\Chrome
        if let Some(local) = dirs::data_local_dir() {
            v.push(local.join("Google").join("Chrome").join("Application").join("chrome.exe"));
            v.push(local.join("Chromium").join("Application").join("chrome.exe"));
        }
        // %APPDATA% (roaming)
        if let Some(roaming) = dirs::data_dir() {
            v.push(roaming.join("Google").join("Chrome").join("Application").join("chrome.exe"));
        }
        v
    };

    #[cfg(target_os = "macos")]
    let system_candidates: Vec<PathBuf> = vec![
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        PathBuf::from("/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary"),
    ];

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let system_candidates: Vec<PathBuf> = vec![
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/google-chrome-stable"),
        PathBuf::from("/usr/bin/chromium-browser"),
        PathBuf::from("/usr/bin/chromium"),
        PathBuf::from("/snap/bin/chromium"),
        PathBuf::from("/usr/local/bin/chromium"),
        PathBuf::from("/usr/local/bin/google-chrome"),
    ];

    for path in &system_candidates {
        if path.exists() {
            logger::info(&format!("Chrome do sistema encontrado: {}", path.display()));
            return Some(path.clone());
        }
    }

    // ── 4. Busca no PATH do sistema ─────────────────────────────────────────
    #[cfg(target_os = "windows")]
    let (finder_cmd, browser_names) = ("where", vec!["chrome.exe", "chromium.exe"]);
    #[cfg(not(target_os = "windows"))]
    let (finder_cmd, browser_names) = (
        "which",
        vec!["google-chrome", "google-chrome-stable", "chromium-browser", "chromium"],
    );

    for name in &browser_names {
        let mut cmd = std::process::Command::new(finder_cmd);
        cmd.arg(name);

        // No Windows (subsystem "windows"), processos filhos de console abrem
        // uma janela de terminal que pisca na tela. CREATE_NO_WINDOW evita isso.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let raw = String::from_utf8_lossy(&output.stdout);
                let first_line = raw.lines().next().unwrap_or("").trim().to_string();
                if !first_line.is_empty() {
                    let p = PathBuf::from(&first_line);
                    if p.exists() {
                        logger::info(&format!("Chrome no PATH: {}", p.display()));
                        return Some(p);
                    }
                }
            }
        }
    }

    // ── 5. Download automático via chromiumoxide fetcher ────────────────────
    logger::warn("Chrome/Chromium não encontrado localmente. Tentando baixar automaticamente...");

    {
        use chromiumoxide::fetcher::{BrowserFetcher, BrowserFetcherOptions};

        match BrowserFetcherOptions::default() {
            Ok(options) => {
                // Timeout: em conexões muito lentas/instáveis o download pode
                // ficar pendurado para sempre e travar a requisição inteira.
                match timeout(CHROMIUM_DOWNLOAD_TIMEOUT, BrowserFetcher::new(options).fetch()).await {
                    Ok(Ok(info)) => {
                        logger::info(&format!(
                            "Chromium baixado com sucesso: {}",
                            info.executable_path.display()
                        ));
                        return Some(info.executable_path);
                    }
                    Ok(Err(e)) => {
                        logger::warn(&format!("Falha ao baixar Chromium automaticamente: {}", e));
                    }
                    Err(_) => {
                        logger::warn("Download do Chromium excedeu o tempo limite (conexão lenta?).");
                    }
                }
            }
            Err(e) => {
                logger::warn(&format!("Falha ao preparar opções do fetcher: {}", e));
            }
        }
    }

    logger::error(
        "Nenhum Chrome/Chromium encontrado. \
        Instale o Google Chrome (https://www.google.com/chrome) \
        ou defina a variável CHROME_PATH com o caminho do executável.",
    );
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// XPath helpers
// ─────────────────────────────────────────────────────────────────────────────

fn get_option_xpath(text: &str) -> String {
    let mut terms = vec![text.to_string()];
    match text {
        "Casa" => terms.push("House".to_string()),
        "Apartamento" => {
            terms.push("Apartment".to_string());
            terms.push("Condo".to_string());
        }
        "Casa geminada" => terms.push("Townhouse".to_string()),
        "À venda" => {
            terms.push("sale".to_string());
            terms.push("Venda".to_string());
            terms.push("For sale".to_string());
        }
        "Aluguel" => {
            terms.push("rent".to_string());
            terms.push("Locação".to_string());
            terms.push("Aluguel".to_string());
            terms.push("For rent".to_string());
        }
        "Carro/picape" => {
            terms.push("Car/Truck".to_string());
            terms.push("Car or pickup".to_string());
            terms.push("Carro".to_string());
            terms.push("Picape".to_string());
            terms.push("Carro/Caminhão".to_string());
        }
        "Motocicleta" => terms.push("Motorcycle".to_string()),
        "Veículos para esportes" => {
            terms.push("Powersport".to_string());
            terms.push("Powersports".to_string());
        }
        "Trailer" => terms.push("Trailer".to_string()),
        "Reboque" => {
            terms.push("Utility trailer".to_string());
            terms.push("reboque".to_string());
        }
        "Barco" => terms.push("Boat".to_string()),
        "Comercial/industrial" => {
            terms.push("Commercial".to_string());
            terms.push("Industrial".to_string());
        }
        "Excelente" => {
            terms.push("Like new".to_string());
            terms.push("excelente".to_string());
        }
        "Muito bom" => {
            terms.push("Very good".to_string());
            terms.push("muito bom".to_string());
        }
        "Bom" => {
            terms.push("Good".to_string());
            terms.push("bom".to_string());
        }
        "Razoável" => {
            terms.push("Fair".to_string());
            terms.push("razoável".to_string());
        }
        "Ruim" => {
            terms.push("Poor".to_string());
            terms.push("ruim".to_string());
        }
        "Gasolina" => {
            terms.push("Gas".to_string());
            terms.push("Gasoline".to_string());
        }
        "Diesel" => terms.push("Diesel".to_string()),
        "Híbrido" => terms.push("Hybrid".to_string()),
        "Híbrido plug-in" => terms.push("Plug-in hybrid".to_string()),
        "Elétrico" => terms.push("Electric".to_string()),
        "Flex" => terms.push("Flex".to_string()),
        "Cupê" => terms.push("Coupe".to_string()),
        "Sedã" => terms.push("Sedan".to_string()),
        "Hatch" => terms.push("Hatchback".to_string()),
        "SUV" => terms.push("SUV".to_string()),
        "Conversível" => terms.push("Convertible".to_string()),
        "Station wagon" => {
            terms.push("Wagon".to_string());
            terms.push("Station".to_string());
        }
        "Minivan" => terms.push("Minivan".to_string()),
        "Carro compacto" => {
            terms.push("Compact".to_string());
            terms.push("Compact car".to_string());
        }
        "Outro" => terms.push("Other".to_string()),
        _ => {}
    }

    let conditions: Vec<String> = terms
        .into_iter()
        .map(|t| format!("contains(., '{}')", t))
        .collect();

    format!("//*[@role='option'][{}]", conditions.join(" or "))
}

// ─────────────────────────────────────────────────────────────────────────────
// Extensão de Page
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait PageExt {
    async fn wait_for_element(&self, selector: &str) -> Result<Element, DomainError>;
    async fn wait_for_xpath(&self, xpath: &str) -> Result<Element, DomainError>;
    async fn click_option(&self, text: &str) -> Result<(), DomainError>;
    async fn focus_and_type(&self, xpath: &str, value: &str) -> Result<(), DomainError>;
    async fn select_dropdown(&self, xpath: &str, option_text: &str) -> Result<(), DomainError>;
    async fn upload_files(&self, selector: &str, files: Vec<String>) -> Result<(), DomainError>;
}

#[async_trait]
impl PageExt for Page {
    async fn wait_for_element(&self, selector: &str) -> Result<Element, DomainError> {
        for _ in 1..=40 {
            if let Ok(el) = self.find_element(selector).await {
                return Ok(el);
            }
            sleep(Duration::from_millis(750)).await;
        }

        let current_url = self
            .evaluate("window.location.href")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .unwrap_or_else(|| "(não foi possível obter a URL)".to_string());

        logger::error(&format!(
            "Elemento não encontrado após 30s: {} | Página atual: {}",
            selector, current_url
        ));

        Err(DomainError::AutomationError(format!(
            "Elemento não carregou na tela: {}",
            selector
        )))
    }

    async fn wait_for_xpath(&self, xpath: &str) -> Result<Element, DomainError> {
        for _ in 1..=40 {
            if let Ok(el) = self.find_xpath(xpath).await {
                return Ok(el);
            }
            sleep(Duration::from_millis(750)).await;
        }

        let current_url = self
            .evaluate("window.location.href")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .unwrap_or_else(|| "(não foi possível obter a URL)".to_string());

        logger::error(&format!(
            "XPath não encontrado após 30s: {} | Página atual: {}",
            xpath, current_url
        ));

        Err(DomainError::AutomationError(format!(
            "XPath não carregou na tela: {}",
            xpath
        )))
    }

    async fn click_option(&self, text: &str) -> Result<(), DomainError> {
        let xpath = get_option_xpath(text);
        let el = self.wait_for_xpath(&xpath).await?;
        if el.click().await.is_err() {
            let click_js = format!(
                r#"(function() {{
                    var el = document.evaluate({:?}, document, null,
                        XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
                    if (el) {{ el.click(); return true; }}
                    return false;
                }})()"#,
                xpath
            );
            let _ = self.evaluate(click_js).await;
        }
        sleep(Duration::from_millis(300)).await;
        Ok(())
    }

    async fn focus_and_type(&self, xpath: &str, value: &str) -> Result<(), DomainError> {
        let el = self.wait_for_xpath(xpath).await?;
        if el.click().await.is_err() {
            let focus_js = format!(
                r#"(function() {{
                    var el = document.evaluate({:?}, document, null,
                        XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
                    if (el) {{ el.focus(); el.click(); return true; }}
                    return false;
                }})()"#,
                xpath
            );
            let _ = self.evaluate(focus_js).await;
        }

        let js = format!(
            r#"(function() {{
                var el = document.evaluate({:?}, document, null,
                    XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
                if (!el) return false;
                el.focus();
                var success = document.execCommand('insertText', false, {:?});
                if (!success || el.value !== {:?}) {{
                    el.value = {:?};
                    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                }}
                return true;
            }})()"#,
            xpath, value, value, value
        );

        let ok = self
            .evaluate(js)
            .await
            .map_err(|e| {
                DomainError::AutomationError(format!(
                    "Falha ao executar script de digitação ({}): {}",
                    xpath, e
                ))
            })?
            .into_value::<bool>()
            .unwrap_or(false);

        if !ok {
            return Err(DomainError::AutomationError(format!(
                "O JS de digitação falhou no elemento: {}",
                xpath
            )));
        }

        Ok(())
    }

    async fn select_dropdown(&self, xpath: &str, option_text: &str) -> Result<(), DomainError> {
        let el = self.wait_for_xpath(xpath).await?;
        if el.click().await.is_err() {
            let click_js = format!(
                r#"(function() {{
                    var el = document.evaluate({:?}, document, null,
                        XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
                    if (el) {{ el.click(); return true; }}
                    return false;
                }})()"#,
                xpath
            );
            let _ = self.evaluate(click_js).await;
        }
        sleep(Duration::from_secs(1)).await;
        self.click_option(option_text).await?;
        Ok(())
    }

    /// Envia arquivos para um `<input type="file">` de forma robusta.
    ///
    /// O Facebook é uma SPA em React que re-renderiza o input de fotos com
    /// frequência. Quando isso acontece, o `nodeId` capturado anteriormente
    /// fica obsoleto e o `DOM.setFileInputFiles` falha com
    /// `-32602: Invalid parameters` ("Could not find node with given id").
    ///
    /// Para evitar isso:
    ///   1. Reencontramos o elemento a cada tentativa (nó sempre atual).
    ///   2. Usamos `backend_node_id` em vez de `node_id` — o backend node id
    ///      é estável e sobrevive aos re-renders do React.
    ///   3. Repetimos algumas vezes em caso de falha transitória.
    async fn upload_files(&self, selector: &str, files: Vec<String>) -> Result<(), DomainError> {
        // Garante que só enviamos caminhos que realmente existem no disco —
        // um caminho inexistente também provoca erro no Chromium.
        let valid_files: Vec<String> = files
            .into_iter()
            .filter(|p| std::path::Path::new(p).exists())
            .collect();

        if valid_files.is_empty() {
            return Err(DomainError::AutomationError(
                "Nenhuma foto válida foi encontrada para enviar (os arquivos podem ter falhado no download/otimização).".to_string(),
            ));
        }

        let mut last_error = String::from("desconhecido");

        for attempt in 1..=5 {
            // Reencontra o input a cada tentativa para obter um nó atual.
            let el = match self.find_element(selector).await {
                Ok(el) => el,
                Err(e) => {
                    last_error = e.to_string();
                    sleep(Duration::from_millis(800)).await;
                    continue;
                }
            };

            let result = self
                .execute(SetFileInputFilesParams {
                    files: valid_files.clone(),
                    // Preferimos backend_node_id (estável) a node_id (volátil).
                    node_id: None,
                    backend_node_id: Some(el.backend_node_id),
                    object_id: None,
                })
                .await;

            match result {
                Ok(_) => {
                    if attempt > 1 {
                        logger::info(&format!(
                            "Fotos enviadas ao Chromium na tentativa {}.",
                            attempt
                        ));
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_error = e.to_string();
                    logger::warn(&format!(
                        "Falha ao enviar fotos (tentativa {}/5): {} — tentando novamente...",
                        attempt, last_error
                    ));
                    sleep(Duration::from_millis(800)).await;
                }
            }
        }

        Err(DomainError::AutomationError(format!(
            "Falha ao enviar as fotos para o Chromium: {}",
            last_error
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BrowserGuard — garante shutdown gracioso preservando cookies
// ─────────────────────────────────────────────────────────────────────────────

struct BrowserGuard {
    browser: Option<Browser>,
}

impl BrowserGuard {
    fn new(browser: Browser) -> Self {
        Self {
            browser: Some(browser),
        }
    }

    async fn close(mut self) {
        if let Some(mut browser) = self.browser.take() {
            let _ = browser.close().await;
            let _ = browser.wait().await;
            sleep(Duration::from_secs(1)).await;
        }
    }
}

impl Drop for BrowserGuard {
    fn drop(&mut self) {
        if let Some(mut browser) = self.browser.take() {
            tokio::task::spawn(async move {
                let _ = browser.close().await;
                let _ = browser.wait().await;
            });
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lançamento do browser
// ─────────────────────────────────────────────────────────────────────────────

/// Flags de lançamento progressivamente mais permissivos.
/// Cada conjunto é tentado em sequência até o browser iniciar com sucesso.
fn headed_flag_sets() -> Vec<Vec<&'static str>> {
    vec![
        // Tentativa 1: flags normais
        vec![
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-setuid-sandbox",
            "--disable-gpu",
            "--disable-infobars",
            "--disable-notifications",
            "--disable-blink-features=AutomationControlled",
        ],
        // Tentativa 2: mais permissivo (ambientes restritos)
        vec![
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-setuid-sandbox",
            "--disable-gpu",
            "--no-zygote",
            "--disable-infobars",
            "--disable-notifications",
            "--disable-blink-features=AutomationControlled",
        ],
        // Tentativa 3: single-process como último recurso
        vec![
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-setuid-sandbox",
            "--disable-gpu",
            "--no-zygote",
            "--single-process",
            "--disable-infobars",
            "--disable-notifications",
        ],
    ]
}

fn headless_flag_sets() -> Vec<Vec<&'static str>> {
    vec![
        // Tentativa 1
        vec![
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-setuid-sandbox",
            "--disable-gpu",
        ],
        // Tentativa 2
        vec![
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-setuid-sandbox",
            "--disable-gpu",
            "--no-zygote",
        ],
        // Tentativa 3
        vec![
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-setuid-sandbox",
            "--disable-gpu",
            "--no-zygote",
            "--single-process",
        ],
    ]
}

fn chrome_not_found_error() -> DomainError {
    DomainError::AutomationError(
        "Não foi possível iniciar o navegador. \
        Verifique se o Google Chrome está instalado no computador. \
        Caso esteja instalado e o erro persistir, tente reiniciar o aplicativo ou \
        desativar temporariamente o antivírus."
            .to_string(),
    )
}

pub struct FacebookMarketplaceService {
    /// Serializa as operações de browser. Sem isso, duas requisições
    /// simultâneas abrem dois Chromes no mesmo perfil (corrompendo a sessão
    /// via SingletonLock) e sobrecarregam máquinas fracas.
    operation_lock: Mutex<()>,
}

impl FacebookMarketplaceService {
    pub fn new() -> Self {
        Self {
            operation_lock: Mutex::new(()),
        }
    }

    /// Tenta adquirir o lock de operação. Se outra operação estiver em
    /// andamento, falha imediatamente com uma mensagem clara em vez de
    /// enfileirar (o cliente HTTP estouraria timeout esperando).
    fn acquire_operation_lock(&self) -> Result<MutexGuard<'_, ()>, DomainError> {
        self.operation_lock.try_lock().map_err(|_| {
            DomainError::AutomationError(
                "Já existe uma operação em andamento no navegador. \
                Aguarde ela terminar antes de iniciar outra."
                    .to_string(),
            )
        })
    }

    async fn launch_browser(client_id: &str) -> Result<Browser, DomainError> {
        let chrome_path = find_chrome_executable().await;

        let extra_flags: Vec<&str> = vec![
            "--start-maximized",
            "--window-size=1280,720",
            "--disable-infobars",
            "--disable-notifications",
            "--disable-blink-features=AutomationControlled",
            "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
            "--no-restore-session-state",
            "--restore-last-session=false",
            "--disable-session-crashed-bubble",
            "--disable-background-mode",
            "--password-store=basic",
            "--use-mock-keychain",
            "--lang=pt-BR",
            "--disable-ipc-flooding-protection",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            "--disable-client-side-phishing-detection",
            "--disable-crash-reporter",
            "--disable-oopr-debug-crash-dump",
            "--no-crash-upload",
            "--hide-crash-restore-bubble",
            "--suppress-message-center-popups",
            "--disable-popup-blocking",
            "--no-first-run",
            "--no-default-browser-check",
            "--new-window",
        ];

        for (attempt, base_flags) in headed_flag_sets().into_iter().enumerate() {
            let mut builder = BrowserConfig::builder()
                .with_head()
                .user_data_dir(profile_dir(client_id))
                .viewport(chromiumoxide::handler::viewport::Viewport {
                    width: 1280,
                    height: 720,
                    device_scale_factor: Some(1.0),
                    emulating_mobile: false,
                    is_landscape: true,
                    has_touch: false,
                });

            if let Some(ref path) = chrome_path {
                builder = builder.chrome_executable(path.clone());
            }

            for flag in base_flags.iter().chain(extra_flags.iter()) {
                builder = builder.arg(*flag);
            }

            match builder.build() {
                Err(e) => {
                    logger::warn(&format!("Falha ao construir configuração do browser (tentativa {}): {}", attempt + 1, e));
                    continue;
                }
                Ok(config) => {
                    // Timeout: antivírus escaneando o binário ou disco lento
                    // podem deixar o launch pendurado indefinidamente.
                    match timeout(BROWSER_LAUNCH_TIMEOUT, Browser::launch(config)).await {
                        Ok(Ok((browser, mut handler))) => {
                            tokio::task::spawn(async move {
                                while let Some(h) = handler.next().await {
                                    if h.is_err() { break; }
                                }
                            });
                            logger::info(&format!("Browser iniciado (tentativa {})", attempt + 1));
                            return Ok(browser);
                        }
                        Ok(Err(e)) => {
                            logger::warn(&format!(
                                "Tentativa {} falhou ao iniciar o browser: {:?}",
                                attempt + 1, e
                            ));
                            // Aguarda um pouco antes de tentar novamente
                            sleep(Duration::from_millis(500)).await;
                        }
                        Err(_) => {
                            logger::warn(&format!(
                                "Tentativa {}: browser não iniciou em {}s (timeout)",
                                attempt + 1,
                                BROWSER_LAUNCH_TIMEOUT.as_secs()
                            ));
                        }
                    }
                }
            }
        }

        logger::error("Todas as tentativas de iniciar o browser falharam.");
        Err(chrome_not_found_error())
    }

    async fn launch_browser_headless(client_id: &str) -> Result<Browser, DomainError> {
        let chrome_path = find_chrome_executable().await;

        for (attempt, base_flags) in headless_flag_sets().into_iter().enumerate() {
            let mut builder = BrowserConfig::builder()
                .user_data_dir(profile_dir(client_id));

            if let Some(ref path) = chrome_path {
                builder = builder.chrome_executable(path.clone());
            }

            for flag in &base_flags {
                builder = builder.arg(*flag);
            }

            match builder.build() {
                Err(e) => {
                    logger::warn(&format!("Falha ao construir config headless (tentativa {}): {}", attempt + 1, e));
                    continue;
                }
                Ok(config) => {
                    match timeout(BROWSER_LAUNCH_TIMEOUT, Browser::launch(config)).await {
                        Ok(Ok((browser, mut handler))) => {
                            tokio::task::spawn(async move {
                                while let Some(h) = handler.next().await {
                                    if h.is_err() { break; }
                                }
                            });
                            logger::info(&format!("Browser headless iniciado (tentativa {})", attempt + 1));
                            return Ok(browser);
                        }
                        Ok(Err(e)) => {
                            logger::warn(&format!(
                                "Tentativa headless {} falhou: {:?}",
                                attempt + 1, e
                            ));
                            sleep(Duration::from_millis(500)).await;
                        }
                        Err(_) => {
                            logger::warn(&format!(
                                "Tentativa headless {}: browser não iniciou em {}s (timeout)",
                                attempt + 1,
                                BROWSER_LAUNCH_TIMEOUT.as_secs()
                            ));
                        }
                    }
                }
            }
        }

        logger::error("Todas as tentativas headless falharam.");
        Err(chrome_not_found_error())
    }

    async fn get_or_create_page(browser: &Browser, url: &str) -> Result<Page, DomainError> {
        let mut page = None;
        for _ in 0..15 {
            if let Ok(pages) = browser.pages().await {
                if !pages.is_empty() {
                    page = Some(pages.into_iter().next().unwrap());
                    break;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }

        let page = match page {
            Some(p) => {
                p.goto(url).await.map_err(|e| {
                    DomainError::AutomationError(format!("Falha ao navegar para a página: {}", e))
                })?;
                p
            }
            None => browser.new_page(url).await.map_err(|e| {
                DomainError::AutomationError(format!("Falha ao criar nova página: {}", e))
            })?,
        };

        Ok(page)
    }
}

impl Default for FacebookMarketplaceService {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Script anti-detecção
// ─────────────────────────────────────────────────────────────────────────────

const ANTI_DETECTION_JS: &str = r#"
    Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
        configurable: true
    });
    delete window.cdc_adoQpoasnfa76pfcZLmcfl_Array;
    delete window.cdc_adoQpoasnfa76pfcZLmcfl_Promise;
    delete window.cdc_adoQpoasnfa76pfcZLmcfl_Symbol;
    Object.defineProperty(navigator, 'plugins', {
        get: () => [1, 2, 3, 4, 5],
    });
    Object.defineProperty(navigator, 'languages', {
        get: () => ['pt-BR', 'pt', 'en-US', 'en'],
    });
"#;

// ─────────────────────────────────────────────────────────────────────────────
// Implementação dos use cases
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
impl WebscrapingMarketplaceService for FacebookMarketplaceService {
    async fn add_property(&self, entity: Property, client_id: String) -> Result<(), DomainError> {
        sanitize_client_id(&client_id)?;
        let _op = self.acquire_operation_lock()?;

        const XPATH_MODEL_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'venda ou locação') or contains(., 'Home for sale or rent') or contains(., 'Home for sale') or contains(., 'Property for rent') or contains(., 'Property for sale') or contains(., 'Listing type') or contains(., 'Alquiler')]";
        const XPATH_CATEGORY_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Tipo de imóvel') or contains(., 'Home type') or contains(., 'Property type') or contains(., 'Tipo de propiedad')]";
        const XPATH_PARKING_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Vagas de estacionamento') or contains(., 'Parking spaces') or contains(., 'Parking') or contains(., 'Plazas de aparcamiento') or contains(., 'Estacionamiento')]";
        const XPATH_BEDROOM_INPUT: &str = "//span[contains(., 'Número de quartos') or contains(., 'Number of bedrooms') or contains(., 'Bedrooms') or contains(., 'Habitaciones') or contains(., 'Quartos')]/following::input[1]";
        const XPATH_BATHROOM_INPUT: &str = "//span[contains(., 'Número de banheiros') or contains(., 'Number of bathrooms') or contains(., 'Bathrooms') or contains(., 'Baños') or contains(., 'Banheiros')]/following::input[1]";
        const XPATH_PRICE_INPUT: &str = "//span[contains(., 'Preço') or contains(., 'Price') or contains(., 'Precio')]/following::input[1]";
        const XPATH_ADDRESS_INPUT: &str = "//input[@role='combobox'][@aria-autocomplete='list'][not(contains(@aria-label, 'Pesquisar'))][not(contains(@aria-label, 'Search'))]";
        const XPATH_DESCRIPTION_TEXTAREA: &str = "//span[contains(., 'Descrição do imóvel') or contains(., 'Descrição') or contains(., 'Property description') or contains(., 'Description') or contains(., 'Descripción')]/following::textarea[1]";
        const XPATH_METER_INPUT: &str = "//span[contains(., 'Metros quadrados') or contains(., 'Área útil') or contains(., 'Square feet') or contains(., 'Square meters') or contains(., 'Metros cuadrados')]/following::input[1]";
        const XPATH_TAX_INPUT: &str = "//span[contains(., 'Imposto') or contains(., 'Tax') or contains(., 'Impuesto')]/following::input[1]";
        const XPATH_CONDOMINIUM_INPUT: &str = "//span[contains(., 'Condomínio') or contains(., 'Condo') or contains(., 'HOA fee') or contains(., 'HOA') or contains(., 'Condominio')]/following::input[1]";

        let url = "https://www.facebook.com/marketplace/create/rental";

        let browser = Self::launch_browser(client_id.as_str()).await?;
        let guard = BrowserGuard::new(browser);
        let page = Self::get_or_create_page(
            guard.browser.as_ref().ok_or(DomainError::AutomationError(
                "Browser não disponível".to_string(),
            ))?,
            url,
        )
        .await?;

        page.evaluate(
            r#"document.cookie = "locale=pt_BR; domain=.facebook.com; path=/; max-age=31536000; SameSite=None; Secure";"#
        ).await.ok();
        page.evaluate("window.location.reload()").await.ok();
        logger::info("Forçando idioma português no Facebook...");
        sleep(Duration::from_secs(4)).await;

        page.evaluate(ANTI_DETECTION_JS).await.map_err(|e| {
            DomainError::AutomationError(format!("Falha ao aplicar script anti-detecção: {}", e))
        })?;

        sleep(Duration::from_secs(3)).await;
        let current_url = page
            .evaluate("window.location.href")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .unwrap_or_default();

        if current_url.contains("login") || current_url.contains("checkpoint") {
            logger::error(&format!(
                "Usuário não está logado no Facebook. URL atual: {}",
                current_url
            ));
            guard.close().await;
            return Err(DomainError::AutomationError(
                "Você precisa estar logado no Facebook antes de publicar. Use a opção 'Entrar' primeiro.".to_string(),
            ));
        }

        // Garante que o input de fotos exista antes de tentar o upload.
        page.wait_for_element(SEL_PHOTO_INPUT).await?;
        let image_paths: Vec<String> = entity.image().iter().map(|s| s.to_string()).collect();

        page.upload_files(SEL_PHOTO_INPUT, image_paths).await?;

        logger::info("Fotos enviadas, aguardando formulário carregar...");
        sleep(Duration::from_secs(5)).await;

        page.select_dropdown(XPATH_MODEL_DROPDOWN, PropertyModel::transform(entity.model())).await?;
        page.select_dropdown(XPATH_CATEGORY_DROPDOWN, PropertyCategory::transform(entity.category())).await?;

        page.focus_and_type(XPATH_BEDROOM_INPUT, &entity.bedroom().to_string()).await?;
        page.focus_and_type(XPATH_BATHROOM_INPUT, &entity.bathroom().to_string()).await?;
        page.focus_and_type(XPATH_PRICE_INPUT, &entity.price().to_string()).await?;

        page.focus_and_type(XPATH_ADDRESS_INPUT, entity.address()).await?;
        sleep(Duration::from_millis(800)).await;

        if let Ok(el) = page.find_xpath("//*[@role='option'][1]").await {
            let _ = el.click().await;
        }

        page.focus_and_type(XPATH_DESCRIPTION_TEXTAREA, entity.description()).await?;
        page.focus_and_type(XPATH_METER_INPUT, &entity.meter().to_string()).await?;
        page.focus_and_type(XPATH_TAX_INPUT, &entity.tax().to_string()).await?;
        page.focus_and_type(XPATH_CONDOMINIUM_INPUT, &entity.condominium().to_string()).await?;
        page.select_dropdown(XPATH_PARKING_DROPDOWN, &entity.parking().to_string()).await?;

        let mut success = false;
        for _ in 0..240 {
            sleep(Duration::from_secs(2)).await;
            match page.evaluate("window.location.href").await {
                Err(_) => break,
                Ok(js_result) => {
                    if let Ok(url) = js_result.into_value::<String>() {
                        if url.contains("marketplace/you/selling") || url.contains("marketplace/you/vehicles") {
                            success = true;
                            break;
                        }
                    }
                }
            }
        }

        guard.close().await;

        if !success {
            return Err(DomainError::AutomationError(
                "A publicação falhou, o tempo esgotou ou a janela foi fechada prematuramente.".to_string(),
            ));
        }

        Ok(())
    }

    async fn add_vehicle(&self, entity: Vehicle, client_id: String) -> Result<(), DomainError> {
        sanitize_client_id(&client_id)?;
        let _op = self.acquire_operation_lock()?;

        const XPATH_TYPE_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Tipo de veículo') or contains(., 'Vehicle type') or contains(., 'Type') or contains(., 'Tipo de vehículo')]";
        const XPATH_YEAR_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Ano') or contains(., 'Year') or contains(., 'Año')]";
        const XPATH_MAKE_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Fabricante') or contains(., 'Make') or contains(., 'Marca')]";
        const XPATH_MAKE_INPUT: &str = "//span[contains(., 'Fabricante') or contains(., 'Make') or contains(., 'Marca')]/following::input[1]";
        const XPATH_MODEL_INPUT: &str = "//span[contains(., 'Modelo') or contains(., 'Model')]/following::input[1]";
        const XPATH_MILEAGE_INPUT: &str = "//span[contains(., 'Quilometragem') or contains(., 'Mileage') or contains(., 'Kilometraje') or contains(., 'Odometer')]/following::input[1]";
        const XPATH_PRICE_INPUT: &str = "//span[contains(., 'Preço') or contains(., 'Price') or contains(., 'Precio')]/following::input[1]";
        const XPATH_BODYSTYLE_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Estilo da carroceria') or contains(., 'Body style') or contains(., 'Body Style') or contains(., 'Carrocería')]";
        const XPATH_CONDITION_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Condição do veículo') or contains(., 'Condição') or contains(., 'Condition') or contains(., 'Condición')]";
        const XPATH_FUEL_DROPDOWN: &str = "//label[@role='combobox'][contains(., 'Tipo de combustível') or contains(., 'Fuel type') or contains(., 'Fuel') or contains(., 'Combustible')]";
        const XPATH_LOCATION_INPUT: &str = "//input[@role='combobox'][@aria-label='Localização' or @aria-label='Location' or @aria-label='Ubicación']";
        const XPATH_DESCRIPTION_TEXTAREA: &str = "//span[contains(., 'Descrição') or contains(., 'Description') or contains(., 'Descripción')]/following::textarea[1]";
        const SEL_PHOTO_INPUT: &str = "input[type='file'][accept*='image']";

        let url = "https://www.facebook.com/marketplace/create/vehicle";

        let browser = Self::launch_browser(client_id.as_str()).await?;
        let guard = BrowserGuard::new(browser);
        let page = Self::get_or_create_page(
            guard.browser.as_ref().ok_or(DomainError::AutomationError(
                "Browser não disponível".to_string(),
            ))?,
            url,
        )
        .await?;

        page.evaluate(
            r#"document.cookie = "locale=pt_BR; domain=.facebook.com; path=/; max-age=31536000; SameSite=None; Secure";"#
        ).await.ok();
        page.evaluate("window.location.reload()").await.ok();
        logger::info("Forçando idioma português no Facebook...");
        sleep(Duration::from_secs(4)).await;

        page.evaluate(ANTI_DETECTION_JS).await.map_err(|e| {
            DomainError::AutomationError(format!("Falha ao aplicar script anti-detecção: {}", e))
        })?;

        sleep(Duration::from_secs(3)).await;
        let current_url = page
            .evaluate("window.location.href")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .unwrap_or_default();

        if current_url.contains("login") || current_url.contains("checkpoint") {
            logger::error(&format!(
                "Usuário não está logado no Facebook. URL atual: {}",
                current_url
            ));
            guard.close().await;
            return Err(DomainError::AutomationError(
                "Você precisa estar logado no Facebook antes de publicar. Use a opção 'Entrar' primeiro.".to_string(),
            ));
        }

        // Garante que o input de fotos exista antes de tentar o upload.
        page.wait_for_element(SEL_PHOTO_INPUT).await?;
        let image_paths: Vec<String> = entity.image().iter().map(|s| s.to_string()).collect();

        page.upload_files(SEL_PHOTO_INPUT, image_paths).await?;

        logger::info("Fotos enviadas, aguardando formulário carregar...");
        sleep(Duration::from_secs(5)).await;

        page.select_dropdown(XPATH_TYPE_DROPDOWN, VehicleCategory::transform(entity.category())).await?;
        sleep(Duration::from_secs(2)).await;
        page.select_dropdown(XPATH_YEAR_DROPDOWN, &entity.year().to_string()).await?;

        match entity.category() {
            VehicleCategory::CarOrPickup
            | VehicleCategory::Motorcycle
            | VehicleCategory::CommercialOrIndustrial => {
                page.select_dropdown(XPATH_MAKE_DROPDOWN, VehicleManufacturer::transform(entity.manufacturer())).await?;
            }
            _ => {
                page.focus_and_type(XPATH_MAKE_INPUT, VehicleManufacturer::transform(entity.manufacturer())).await?;
            }
        }

        page.focus_and_type(XPATH_MODEL_INPUT, &entity.model()).await?;

        if page.find_xpath(XPATH_MILEAGE_INPUT).await.is_ok() {
            let _ = page.focus_and_type(XPATH_MILEAGE_INPUT, &entity.mileage().to_string()).await;
        }

        if page.find_xpath(XPATH_BODYSTYLE_DROPDOWN).await.is_ok() {
            let _ = page.select_dropdown(XPATH_BODYSTYLE_DROPDOWN, VehicleBodyStyle::transform(entity.bodystyle())).await;
        }

        if page.find_xpath(XPATH_CONDITION_DROPDOWN).await.is_ok() {
            let _ = page.select_dropdown(XPATH_CONDITION_DROPDOWN, VehicleCondition::transform(entity.condition())).await;
        }

        if page.find_xpath(XPATH_FUEL_DROPDOWN).await.is_ok() {
            let _ = page.select_dropdown(XPATH_FUEL_DROPDOWN, VehicleFuel::transform(entity.fuel())).await;
        }

        page.focus_and_type(XPATH_PRICE_INPUT, &entity.price().to_string()).await?;

        page.focus_and_type(XPATH_LOCATION_INPUT, &entity.address()).await?;
        sleep(Duration::from_millis(800)).await;

        if let Ok(el) = page.find_xpath("//*[@role='option'][1]").await {
            let _ = el.click().await;
        }

        if page.find_xpath(XPATH_DESCRIPTION_TEXTAREA).await.is_ok() {
            let _ = page.focus_and_type(XPATH_DESCRIPTION_TEXTAREA, &entity.description()).await;
        }

        let mut success = false;
        for _ in 0..240 {
            sleep(Duration::from_secs(2)).await;
            match page.evaluate("window.location.href").await {
                Err(_) => break,
                Ok(js_result) => {
                    if let Ok(url) = js_result.into_value::<String>() {
                        if url.contains("marketplace/you/selling") || url.contains("marketplace/you/vehicles") {
                            success = true;
                            break;
                        }
                    }
                }
            }
        }

        guard.close().await;

        if !success {
            return Err(DomainError::AutomationError(
                "A publicação falhou, o tempo esgotou ou a janela foi fechada prematuramente.".to_string(),
            ));
        }

        Ok(())
    }

    async fn signin(&self, client_id: String) -> Result<(), DomainError> {
        sanitize_client_id(&client_id)?;
        let _op = self.acquire_operation_lock()?;
        let browser = Self::launch_browser(client_id.as_str()).await?;
        let guard = BrowserGuard::new(browser);
        let page = Self::get_or_create_page(
            guard.browser.as_ref().ok_or(DomainError::AutomationError(
                "Browser não disponível".to_string(),
            ))?,
            "https://www.facebook.com/login?locale=pt_BR",
        )
        .await?;

        for _ in 0..240 {
            sleep(Duration::from_secs(2)).await;
            if let Ok(js_result) = page.evaluate("window.location.href").await {
                if let Ok(current_url) = js_result.into_value::<String>() {
                    let is_out_of_login = !current_url.contains("login")
                        && !current_url.contains("two_factor")
                        && !current_url.contains("two-factor")
                        && !current_url.contains("save-device")
                        && !current_url.contains("trust");

                    if is_out_of_login && page.find_element(SEL_FACEBOOK_LOGGED_IN).await.is_ok() {
                        let trust_prompt_visible =
                            page.find_element(SEL_FACEBOOK_TRUST_DEVICE).await.is_ok();

                        if trust_prompt_visible {
                            continue;
                        }

                        sleep(Duration::from_secs(8)).await;
                        guard.close().await;
                        sleep(Duration::from_secs(2)).await;
                        return Ok(());
                    }
                }
            }
        }

        Err(DomainError::NotFound)
    }

    async fn signout(&self, client_id: String) -> Result<(), DomainError> {
        sanitize_client_id(&client_id)?;
        let _op = self.acquire_operation_lock()?;
        let browser = Self::launch_browser_headless(client_id.as_str()).await?;
        let guard = BrowserGuard::new(browser);
        let page = Self::get_or_create_page(
            guard.browser.as_ref().ok_or(DomainError::AutomationError(
                "Browser não disponível".to_string(),
            ))?,
            "https://www.facebook.com/?locale=pt_BR",
        )
        .await?;

        sleep(Duration::from_secs(6)).await;

        if page.find_element(SEL_FACEBOOK_LOGGED_IN).await.is_err() {
            guard.close().await;
            return Ok(());
        }

        let _ = page
            .evaluate(
                r#"
                (function() {
                    const form = document.querySelector('form[action*="logout.php"]');
                    if (!form) return { success: false, reason: 'form not found' };
                    const h = form.querySelector('input[name="h"]');
                    const ref_ = form.querySelector('input[name="ref"]');
                    if (!h) return { success: false, reason: 'token h not found' };
                    const params = new URLSearchParams();
                    params.append('h', h.value);
                    params.append('ref', ref_ ? ref_.value : 'mb');
                    fetch('/logout.php?button_location=settings&button_name=logout', {
                        method: 'POST',
                        credentials: 'include',
                        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
                        body: params.toString()
                    });
                    return { success: true, h: h.value };
                })()
            "#,
            )
            .await
            .map_err(|e| {
                DomainError::AutomationError(format!("Falha ao executar logout: {}", e))
            })?;

        for _ in 0..20 {
            sleep(Duration::from_secs(2)).await;
            if let Ok(js_result) = page.evaluate("window.location.href").await {
                if let Ok(current_url) = js_result.into_value::<String>() {
                    if current_url.contains("login")
                        || current_url.contains("logged_out")
                        || current_url.contains("checkpoint")
                        || current_url.contains("accounts/login")
                    {
                        sleep(Duration::from_secs(3)).await;
                        guard.close().await;
                        return Ok(());
                    }

                    if current_url.contains("facebook.com") && !current_url.contains("login") {
                        page.goto("https://www.facebook.com/login?next&prompt=select_account&login_attempt=1&lwv=100&locale=pt_BR")
                            .await
                            .ok();
                        sleep(Duration::from_secs(2)).await;
                        guard.close().await;
                        return Ok(());
                    }
                }
            }
        }

        Err(DomainError::AutomationError(
            "Timeout: logout não foi confirmado".to_string(),
        ))
    }

    async fn get_account(&self, client_id: String) -> Result<bool, DomainError> {
        sanitize_client_id(&client_id)?;
        let _op = self.acquire_operation_lock()?;
        let browser = Self::launch_browser_headless(client_id.as_str()).await?;
        let guard = BrowserGuard::new(browser);

        let page = Self::get_or_create_page(
            guard.browser.as_ref().ok_or(DomainError::AutomationError(
                "Browser não disponível".to_string(),
            ))?,
            "https://www.facebook.com/?locale=pt_BR",
        )
        .await?;

        sleep(Duration::from_secs(6)).await;

        let is_logged_in = page.find_element(SEL_FACEBOOK_LOGGED_IN).await.is_ok();

        guard.close().await;
        Ok(is_logged_in)
    }
}
