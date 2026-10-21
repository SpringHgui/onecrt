import { useState } from "react";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";

export interface ConnectParams {
  name: string;
  host: string;
  port: number;
  username: string;
  password: string;
  keyPath: string;
}

interface Props {
  initial?: Partial<ConnectParams>;
  showName?: boolean;
  title?: string;
  onClose(): void;
  onConnect(p: ConnectParams, saveSession: boolean): void;
}

const AUTH_METHODS = ["PublicKey", "Keyboard Interactive", "Password", "GSSAPI"];

const PROTOCOLS = ["SSH2", "SSH1", "Telnet", "Telnet/TLS", "Local Shell", "RDP", "Serial", "TAPI", "Rlogin", "Raw"];

export default function QuickConnect({ initial, showName, title = "Quick Connect", onClose, onConnect }: Props) {
  const [protocol, setProtocol] = useState("SSH2");
  const [name, setName] = useState(initial?.name ?? "");
  const [host, setHost] = useState(initial?.host ?? "");
  const [port, setPort] = useState(initial?.port ?? 22);
  const [username, setUsername] = useState(initial?.username ?? "");
  const [password, setPassword] = useState(initial?.password ?? "");
  const [keyPath, setKeyPath] = useState(initial?.keyPath ?? "");
  const [methods, setMethods] = useState<string[]>(AUTH_METHODS);
  const [showProps, setShowProps] = useState(true);
  const [saveSession, setSaveSession] = useState(true);
  const [openInTab, setOpenInTab] = useState(true);
  const [showOnStartup, setShowOnStartup] = useState(true);

  const toggleMethod = (m: string) =>
    setMethods((prev) => (prev.includes(m) ? prev.filter((x) => x !== m) : [...prev, m]));

  const connect = () => {
    if (!host.trim()) return;
    onConnect(
      {
        name: name.trim() || host.trim(),
        host: host.trim(),
        port: Number(port) || 22,
        username: username.trim(),
        // 面板收起时不能把已存的口令/密钥路径当成空写回去，否则一次"改端口"就会清空密码
        password,
        keyPath: keyPath.trim(),
      },
      saveSession,
    );
  };

  return (
    <div className="dialog-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog">
        <div className="dialog-titlebar">
          <span>{title}</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <label className="form-row">
            <span>Protocol:</span>
            <select value={protocol} onChange={(e) => setProtocol(e.target.value)}>
              {PROTOCOLS.map((p) => (
                <option key={p} disabled={p !== "SSH2"}>
                  {p}
                </option>
              ))}
            </select>
          </label>
          <div className="hint">当前仅实现 SSH2，其余协议在列表中置灰占位。</div>
          {showName && (
            <label className="form-row">
              <span>Session name:</span>
              <input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
            </label>
          )}
          <label className="form-row">
            <span>Hostname:</span>
            <input value={host} onChange={(e) => setHost(e.target.value)} autoFocus={!showName} />
          </label>
          <div className="form-row">
            <span>Port:</span>
            <input className="input-sm" type="number" value={port} onChange={(e) => setPort(Number(e.target.value))} />
            <span>Firewall:</span>
            <select defaultValue="None">
              <option>None</option>
            </select>
          </div>
          <label className="form-row">
            <span>Username:</span>
            <input value={username} onChange={(e) => setUsername(e.target.value)} />
          </label>
          <fieldset className="auth-group">
            <legend>Authentication</legend>
            <div className="auth-list">
              {AUTH_METHODS.map((m) => (
                <label key={m} className="auth-item">
                  <input type="checkbox" checked={methods.includes(m)} onChange={() => toggleMethod(m)} />
                  <span>{m}</span>
                </label>
              ))}
            </div>
            <button className="btn" disabled={!methods.length} onClick={() => setShowProps((v) => !v)}>
              Properties...
            </button>
          </fieldset>
          {showProps && (
            <div className="auth-props">
              {methods.includes("Password") && (
                <label className="form-row">
                  <span>Password:</span>
                  <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
                </label>
              )}
              {methods.includes("PublicKey") && (
                <label className="form-row">
                  <span>Private key:</span>
                  <input
                    placeholder="C:\\Users\\me\\.ssh\\id_ed25519 或 ~\\.ssh\\id_rsa"
                    value={keyPath}
                    onChange={(e) => setKeyPath(e.target.value)}
                  />
                </label>
              )}
            </div>
          )}
          <div className="check-rows">
            <label>
              <input type="checkbox" checked={showOnStartup} onChange={(e) => setShowOnStartup(e.target.checked)} />
              Show quick connect on startup
            </label>
            <label>
              <input type="checkbox" checked={saveSession} onChange={(e) => setSaveSession(e.target.checked)} />
              Save session
            </label>
            <label>
              <input type="checkbox" checked={openInTab} onChange={(e) => setOpenInTab(e.target.checked)} />
              Open in a tab
            </label>
          </div>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" disabled={!host.trim()} onClick={connect}>
            Connect
          </button>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
