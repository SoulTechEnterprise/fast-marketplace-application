// ─── Códigos ANSI para cores no terminal ────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const MAGENTA: &str = "\x1b[35m";
const WHITE: &str = "\x1b[97m";
const BLUE: &str = "\x1b[34m";

// ─── Log em arquivo (para depurar na máquina do cliente) ─────────────────────

/// Caminho do arquivo de log: %LOCALAPPDATA%/marketplace/logs/app.log no Windows
/// (ou ~/.local/share|Library nos demais sistemas).
fn log_file_path() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    #[cfg(not(target_os = "windows"))]
    let base = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));

    let dir = base.join("marketplace").join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("app.log")
}

/// Tamanho máximo do arquivo de log antes da rotação (5 MB).
/// Sem isso, o app.log cresce indefinidamente e pode encher o disco
/// do cliente ao longo de meses de uso.
const MAX_LOG_SIZE_BYTES: u64 = 5 * 1024 * 1024;

/// Rotaciona o log se exceder o tamanho máximo: app.log → app.log.old
/// (sobrescrevendo o .old anterior). Mantém no máximo ~10 MB em disco.
fn rotate_log_if_needed(path: &std::path::Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_LOG_SIZE_BYTES {
            let old = path.with_extension("log.old");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(path, &old);
        }
    }
}

/// Grava uma linha de log (sem cores ANSI) no arquivo, em modo append.
/// Falhas de escrita são ignoradas para nunca derrubar a aplicação.
fn log_to_file(level: &str, msg: &str) {
    use std::io::Write;
    let path = log_file_path();
    rotate_log_if_needed(&path);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{} [{}] {}", timestamp(), level, msg);
    }
}

// ─── Timestamp formatado ────────────────────────────────────────────────────

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    let total_secs = now.as_secs();
    // UTC-3 (Brasil)
    let adjusted = total_secs as i64 - 3 * 3600;
    let adjusted = if adjusted < 0 {
        (adjusted + 86400) as u64
    } else {
        adjusted as u64
    };

    let secs_today = adjusted % 86400;
    let hours = secs_today / 3600;
    let minutes = (secs_today % 3600) / 60;
    let seconds = secs_today % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

// ─── Funções de log formatadas ──────────────────────────────────────────────

/// Exibe mensagem informativa com ícone verde e timestamp.
pub fn info(msg: &str) {
    println!(
        "  {DIM}{}{RESET}  {GREEN}{BOLD}✅ INFO{RESET}  {WHITE}{}{RESET}",
        timestamp(),
        msg
    );
    log_to_file("INFO", msg);
}

/// Exibe mensagem de aviso com ícone amarelo e timestamp.
pub fn warn(msg: &str) {
    println!(
        "  {DIM}{}{RESET}  {YELLOW}{BOLD}⚠️  WARN{RESET}  {YELLOW}{}{RESET}",
        timestamp(),
        msg
    );
    log_to_file("WARN", msg);
}

/// Exibe mensagem de erro com ícone vermelho e timestamp.
pub fn error(msg: &str) {
    eprintln!(
        "  {DIM}{}{RESET}  {RED}{BOLD}❌ ERRO{RESET}  {RED}{}{RESET}",
        timestamp(),
        msg
    );
    log_to_file("ERRO", msg);
}

/// Exibe progresso de otimização de imagem.
pub fn image_optimized(original_mb: f64, new_mb: f64, quality: u8) {
    println!(
        "  {DIM}{}{RESET}  {MAGENTA}{BOLD}📸 IMG {RESET}  {WHITE}Otimizada: {CYAN}{:.1} MB{RESET} → {GREEN}{:.1} MB{RESET} {DIM}(qualidade: {}%){RESET}",
        timestamp(),
        original_mb,
        new_mb,
        quality
    );
}

/// Exibe quando uma imagem foi redimensionada.
pub fn image_resized(original_mb: f64, new_mb: f64, scale_percent: u32) {
    println!(
        "  {DIM}{}{RESET}  {MAGENTA}{BOLD}📐 IMG {RESET}  {WHITE}Redimensionada: {CYAN}{:.1} MB{RESET} → {GREEN}{:.1} MB{RESET} {DIM}(escala: {}%){RESET}",
        timestamp(),
        original_mb,
        new_mb,
        scale_percent
    );
}

/// Exibe quando uma imagem foi mantida sem alterações.
pub fn image_kept(size_mb: f64) {
    println!(
        "  {DIM}{}{RESET}  {BLUE}{BOLD}📎 IMG {RESET}  {WHITE}Mantida sem alteração: {GREEN}{:.1} MB{RESET}",
        timestamp(),
        size_mb,
    );
}

/// Exibe quando uma imagem não pôde ser processada.
pub fn image_skipped(reason: &str) {
    println!(
        "  {DIM}{}{RESET}  {YELLOW}{BOLD}⏭️  IMG {RESET}  {YELLOW}Ignorada: {}{RESET}",
        timestamp(),
        reason
    );
}

// ─── Formatador de tamanho de arquivo ───────────────────────────────────────

/// Converte bytes para MB com 1 casa decimal.
pub fn bytes_to_mb(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

// ─── Banner de inicialização ────────────────────────────────────────────────

/// Exibe o banner profissional no terminal ao iniciar o app.
pub fn print_banner(version: &str, port: &str) {
    let banner = format!(
        r#"
{CYAN}{BOLD}
  ╔═══════════════════════════════════════════════════════════════════╗
  ║                                                                   ║
  ║   ███████╗ ██████╗ ██╗   ██╗██╗  ████████╗███████╗ ██████╗██╗  ██╗║
  ║   ██╔════╝██╔═══██╗██║   ██║██║  ╚══██╔══╝██╔════╝██╔════╝██║  ██║║
  ║   ███████╗██║   ██║██║   ██║██║     ██║   █████╗  ██║     ███████║║
  ║   ╚════██║██║   ██║██║   ██║██║     ██║   ██╔══╝  ██║     ██╔══██║║
  ║   ███████║╚██████╔╝╚██████╔╝███████╗██║   ███████╗╚██████╗██║  ██║║
  ║   ╚══════╝ ╚═════╝  ╚═════╝ ╚══════╝╚═╝   ╚══════╝ ╚═════╝╚═╝  ╚═╝║
  ║                                                                   ║
  ║   {WHITE}Fast Marketplace — Automação Inteligente{CYAN}                        ║
  ║                                                                   ║
  ╠═══════════════════════════════════════════════════════════════════╣
  ║                                                                   ║
  ║   {GREEN}✅ Versão:{RESET}     {WHITE}{BOLD}{version}{RESET}{CYAN}                                          ║
  ║   {BLUE}🌐 Endereço:{RESET}   {WHITE}{BOLD}http://127.0.0.1:{port}{RESET}{CYAN}                           ║
  ║   {MAGENTA}📊 Status:{RESET}     {GREEN}{BOLD}Pronto para receber conexões{RESET}{CYAN}                     ║
  ║   {MAGENTA}📸 Imagens:{RESET}    {WHITE}Otimização automática ativada (max 10 MB){RESET}{CYAN}   ║
  ║                                                                   ║
  ║   {DIM}{WHITE}💡 Dica: Mantenha esta janela aberta enquanto usa o app{RESET}{CYAN}      ║
  ║   {DIM}{WHITE}🛑 Para encerrar: pressione Ctrl+C{RESET}{CYAN}                            ║
  ║                                                                   ║
  ╚═══════════════════════════════════════════════════════════════════╝
{RESET}"#,
    );

    println!("{}", banner);
}

/// Exibe mensagem de encerramento gracioso.
pub fn print_shutdown() {
    println!(
        "\n  {CYAN}{BOLD}╔═══════════════════════════════════════════════════╗{RESET}"
    );
    println!(
        "  {CYAN}{BOLD}║{RESET}  {YELLOW}🛑 Encerrando o servidor...{RESET}                      {CYAN}{BOLD}║{RESET}"
    );
    println!(
        "  {CYAN}{BOLD}║{RESET}  {GREEN}✅ Obrigado por usar o Fast Marketplace!{RESET}          {CYAN}{BOLD}║{RESET}"
    );
    println!(
        "  {CYAN}{BOLD}╚═══════════════════════════════════════════════════╝{RESET}\n"
    );
}

/// Exibe separador visual no terminal.
pub fn separator() {
    println!(
        "  {DIM}───────────────────────────────────────────────────────{RESET}"
    );
}
