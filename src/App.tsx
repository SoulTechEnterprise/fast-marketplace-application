import { useEffect, useState } from "react";
import "./App.css";

const API_BASE = "http://127.0.0.1:15137";
const POLL_INTERVAL = 1500;

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

  const cfg = STATUS_CONFIG[status.kind] ?? STATUS_CONFIG.standby;

  return (
    <main className="status-root">
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
            {cfg.label}
          </span>
          <span className="status-message">{status.message}</span>
        </div>

        <div className={`status-dot ${connected ? "dot-on" : "dot-off"}`} title={connected ? "Conectado" : "Sem conexão"} />
      </div>
    </main>
  );
}

export default App;
