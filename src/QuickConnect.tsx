import { useState } from "react";
import { IconCaretDown, IconCaretUp, IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { AuthMethod, defaultAuth } from "./types";
import AuthPropertiesDialog from "./AuthProperties";
import { useI18n } from "./i18n";

export interface ConnectParams {
  name: string;
  host: string;
  port: number;
  username: string;
  /** 本次连接用的明文口令；storePassword 为假时为空，连接前由口令对话框补 */
  password: string;
  storePassword: boolean;
  keyPath: string;
  auth: AuthMethod[];
}

interface Props {
  initial?: Partial<ConnectParams>;
  showName?: boolean;
  title?: string;
  onClose(): void;
  onConnect(p: ConnectParams, saveSession: boolean): void;
}

const PROTOCOLS = ["SSH2", "SSH1", "Telnet", "Telnet/TLS", "Local Shell", "RDP", "Serial", "TAPI", "Rlogin", "Raw"];

export default function QuickConnect({ initial, showName, title = "Quick Connect", onClose, onConnect }: Props) {
  const { t } = useI18n();
  const [protocol, setProtocol] = useState("SSH2");
  const [name, setName] = useState(initial?.name ?? "");
  const [host, setHost] = useState(initial?.host ?? "");
  const [port, setPort] = useState(initial?.port ?? 22);
  const [username, setUsername] = useState(initial?.username ?? "");
  const [password, setPassword] = useState(initial?.password ?? "");
  const [storePassword, setStorePassword] = useState(initial?.storePassword ?? !!initial?.password);
  const [keyPath, setKeyPath] = useState(initial?.keyPath ?? "");
  const [auth, setAuth] = useState<AuthMethod[]>(initial?.auth?.length ? initial.auth : defaultAuth());
  const [selected, setSelected] = useState<string | null>(auth[0]?.method ?? null);
  const [propsFor, setPropsFor] = useState<string | null>(null);
  const [saveSession, setSaveSession] = useState(true);
  const [openInTab, setOpenInTab] = useState(true);
  const [showOnStartup, setShowOnStartup] = useState(true);

  const selIndex = auth.findIndex((a) => a.method === selected);

  const toggleMethod = (m: string) =>
    setAuth((prev) => prev.map((a) => (a.method === m ? { ...a, enabled: !a.enabled } : a)));

  const move = (delta: number) =>
    setAuth((prev) => {
      const i = prev.findIndex((a) => a.method === selected);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= prev.length) return prev;
      const next = [...prev];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });

  const connect = () => {
    if (!host.trim()) return;
    onConnect(
      {
        name: name.trim() || host.trim(),
        host: host.trim(),
        port: Number(port) || 22,
        username: username.trim(),
        password: storePassword ? password : "",
        storePassword,
        keyPath: keyPath.trim(),
        auth,
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
            <span>{t("qc.protocol")}</span>
            <select value={protocol} onChange={(e) => setProtocol(e.target.value)}>
              {PROTOCOLS.map((p) => (
                <option key={p} disabled={p !== "SSH2"}>
                  {p}
                </option>
              ))}
            </select>
          </label>
          <div className="hint">{t("qc.hint")}</div>
          {showName && (
            <label className="form-row">
              <span>{t("qc.sessionName")}</span>
              <input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
            </label>
          )}
          <label className="form-row">
            <span>{t("qc.hostname")}</span>
            <input value={host} onChange={(e) => setHost(e.target.value)} autoFocus={!showName} />
          </label>
          <div className="form-row">
            <span>{t("qc.port")}</span>
            <input className="input-sm" type="number" value={port} onChange={(e) => setPort(Number(e.target.value))} />
            <span>{t("qc.firewall")}</span>
            <select defaultValue="None">
              <option>{t("qc.none")}</option>
            </select>
          </div>
          <label className="form-row">
            <span>{t("qc.username")}</span>
            <input value={username} onChange={(e) => setUsername(e.target.value)} />
          </label>
          <fieldset className="auth-group">
            <legend>{t("qc.auth")}</legend>
            <div className="auth-list">
              {auth.map((a) => (
                <div
                  key={a.method}
                  className={`auth-item${a.method === selected ? " sel" : ""}`}
                  onClick={() => setSelected(a.method)}
                >
                  <input type="checkbox" checked={a.enabled} onChange={() => toggleMethod(a.method)} />
                  <span>{a.method}</span>
                </div>
              ))}
            </div>
            <div className="auth-arrows">
              <button className="btn-arrow" title={t("qc.moveUp")} disabled={selIndex <= 0} onClick={() => move(-1)}>
                <IconCaretUp {...ink(14)} />
              </button>
              <button
                className="btn-arrow"
                title={t("qc.moveDown")}
                disabled={selIndex < 0 || selIndex >= auth.length - 1}
                onClick={() => move(1)}
              >
                <IconCaretDown {...ink(14)} />
              </button>
            </div>
            <button className="btn" disabled={!selected} onClick={() => selected && setPropsFor(selected)}>
              {t("qc.properties")}
            </button>
          </fieldset>
          <div className="hint auth-note">{t("qc.authNote")}</div>
          <div className="check-rows">
            <label>
              <input type="checkbox" checked={showOnStartup} onChange={(e) => setShowOnStartup(e.target.checked)} />
              {t("qc.showOnStartup")}
            </label>
            <label>
              <input type="checkbox" checked={saveSession} onChange={(e) => setSaveSession(e.target.checked)} />
              {t("qc.saveSession")}
            </label>
            <label>
              <input type="checkbox" checked={openInTab} onChange={(e) => setOpenInTab(e.target.checked)} />
              {t("qc.openInTab")}
            </label>
          </div>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" disabled={!host.trim()} onClick={connect}>
            {t("qc.connect")}
          </button>
          <button className="btn" onClick={onClose}>
            {t("common.cancel")}
          </button>
        </div>
      </div>
      {propsFor && (
        <AuthPropertiesDialog
          method={propsFor}
          password={password}
          storePassword={storePassword}
          keyPath={keyPath}
          onClose={() => setPropsFor(null)}
          onOk={(r) => {
            setPassword(r.password);
            setStorePassword(r.storePassword);
            setKeyPath(r.keyPath);
            setPropsFor(null);
          }}
        />
      )}
    </div>
  );
}
