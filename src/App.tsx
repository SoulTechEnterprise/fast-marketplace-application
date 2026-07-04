import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import "./App.css";

const API_BASE = "http://127.0.0.1:15137";
const POLL_INTERVAL = 1500;

// Repositório das releases e instalador (mesma URL usada no site).
const GITHUB_REPO = "SoulTechEnterprise/fast-marketplace-application";
const LATEST_RELEASE_API = `https://api.github.com/repos/${GITHUB_REPO}/releases/latest`;
const INSTALLER_URL = `https://github.com/${GITHUB_REPO}/releases/latest/download/fast-marketplace_x64-setup.exe`;

type StatusKind =
  | "standby"
  | "verificando"
  | "entrando"
  | "publicando"
  | "publicado"
  | "erro";

interface AppStatus {
  kind: StatusKind;
  message: string;
}

const STATUS_CONFIG: Record<
  StatusKind,
  { label: string; color: string; spinning: boolean; icon: string }
> = {
  standby:    { label: "Standby",     color: "#6b7280", spinning: false, icon: "⏸" },
  verificando:{ label: "Verificando", color: "#3b82f6", spinning: true,  icon: "🔍" },
  entrando:   { label: "Entrando",    color: "#3b82f6", spinning: true,  icon: "🔑" },
  publicando: { label: "Publicando",  color: "#f59e0b", spinning: true,  icon: "📤" },
  publicado:  { label: "Publicado",   color: "#22c55e", spinning: false, icon: "✅" },
  erro:       { label: "Erro",        color: "#ef4444", spinning: false, icon: "❌" },
};

type UpdateState = "idle" | "checking" | "latest" | "available" | "error";

/** Compara duas versões "x.y.z". Retorna true se `a` for mais nova que `b`. */
function isNewer(a: string, b: string): boolean {
  const pa = a.split(".").map((n) => parseInt(n, 10) || 0);
  const pb = b.split(".").map((n) => parseInt(n, 10) || 0);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x > y) return true;
    if (x < y) return false;
  }
  return false;
}

function Spinner({ color }: { color: string }) {
  return (
    <span
      className="spinner"
      style={{ borderTopColor: color, borderRightColor: color }}
    />
  );
}

function App() {
  const [status, setStatus] = useState<AppStatus>({
    kind: "standby",
    message: "Aguardando",
  });
  const [connected, setConnected] = useState(true);

  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [latestVersion, setLatestVersion] = useState<string | null>(null);
  const [updateState, setUpdateState] = useState<UpdateState>("idle");
  const [updateError, setUpdateError] = useState<string>("");

  useEffect(() => {
    let active = true;

    async function poll() {
      try {
        const res = await fetch(`${API_BASE}/status`);
        if (!res.ok) throw new Error("non-ok");
        const data: AppStatus = await res.json();
        if (active) {
          setStatus(data);
          setConnected(true);
        }
      } catch {
        if (active) setConnected(false);
      }
    }

    poll();
    const id = setInterval(poll, POLL_INTERVAL);
    return () => {
      active = false;
      clearInterval(id);
    };
  }, []);

  // Descobre a versão instalada do app (definida em tauri.conf.json).
  useEffect(() => {
    getVersion()
      .then(setCurrentVersion)
      .catch(() => setCurrentVersion(null));
  }, []);

  async function checkForUpdates() {
    setUpdateState("checking");
    setUpdateError("");
    try {
      const res = await fetch(LATEST_RELEASE_API, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!res.ok) throw new Error("non-ok");

      const data: { tag_name?: string } = await res.json();
      const latest = String(data.tag_name ?? "").replace(/^v/i, "").trim();

      if (!latest) throw new Error("sem tag");

      setLatestVersion(latest);

      const current = currentVersion ?? (await getVersion());
      setUpdateState(isNewer(latest, current) ? "available" : "latest");
    } catch {
      setUpdateError("Não foi possível verificar. Tente novamente.");
      setUpdateState("error");
    }
  }

  async function downloadInstaller() {
    try {
      await openUrl(INSTALLER_URL);
    } catch {
      setUpdateError("Não foi possível abrir o download.");
      setUpdateState("error");
    }
  }

  const cfg = STATUS_CONFIG[status.kind] ?? STATUS_CONFIG.standby;

  // Sem conexão com o serviço local: o servidor pode estar reiniciando
  // (ele tenta se recuperar sozinho). Mostra uma mensagem clara ao usuário.
  const displayLabel = connected ? cfg.label : "Reconectando";
  const displayMessage = connected
    ? status.message
    : "Conectando ao serviço local... Se persistir, reinicie o aplicativo.";

  return (
    <main className="status-root">
      <div className="app-shell">
        <div className="status-card" style={{ "--accent": cfg.color } as React.CSSProperties}>
          <div className="status-icon-wrap">
            {cfg.spinning ? (
              <Spinner color={cfg.color} />
            ) : (
              <span className="status-emoji">{cfg.icon}</span>
            )}
          </div>

          <div className="status-text">
            <span className="status-kind" style={{ color: cfg.color }}>
              {displayLabel}
            </span>
            <span className="status-message">{displayMessage}</span>
          </div>

          <div className={`status-dot ${connected ? "dot-on" : "dot-off"}`} title={connected ? "Conectado" : "Sem conexão"} />
        </div>

        <div className="update-card">
          <div className="update-row">
            <span className="update-version">
              Versão {currentVersion ?? "—"}
            </span>
            <button
              type="button"
              className="update-btn"
              onClick={checkForUpdates}
              disabled={updateState === "checking"}
            >
              {updateState === "checking"
                ? "Verificando..."
                : "Verificar atualizações"}
            </button>
          </div>

          {updateState === "latest" && (
            <span className="update-msg update-msg-muted">
              Você está na versão mais recente.
            </span>
          )}

          {updateState === "available" && (
            <div className="update-row">
              <span className="update-msg update-msg-available">
                Nova versão {latestVersion} disponível!
              </span>
              <button
                type="button"
                className="update-btn update-btn-primary"
                onClick={downloadInstaller}
              >
                Baixar instalador
              </button>
            </div>
          )}

          {updateState === "error" && (
            <span className="update-msg update-msg-error">{updateError}</span>
          )}
        </div>
      </div>
    </main>
  );
}

export default App;
