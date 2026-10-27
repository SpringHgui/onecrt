import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCopy, IconRefresh, IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";
import { snippet, useMcp } from "./mcp";

export default function McpDialog({ onClose }: { onClose(): void }) {
  const { t } = useI18n();
  // agent 会话在后台增删，弹窗开着就两秒回一次
  const { st, err, setErr, call } = useMcp(2000);
  const [portText, setPortText] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (st) setPortText(String(st.port));
  }, [st?.port]);

  const copy = (text: string) => {
    // WebView2 里 navigator.clipboard 要弹权限框、窗口失焦还会失败，一律走原生剪贴板
    invoke("clipboard_write", { text }).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }, () => setErr(t("mcp.copyFailed")));
  };

  if (!st) return null;

  return (
    <div
      className="dialog-overlay"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="dialog">
        <div className="dialog-titlebar">
          <span>{t("mcp.title")}</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <p className="pwd-note">{t("mcp.note")}</p>

          <div className="check-rows mcp-enable">
            <label>
              <input
                type="checkbox"
                checked={st.enabled}
                onChange={(e) => call("mcp_set_enabled", { enabled: e.target.checked })}
              />
              {t("mcp.enable")}
            </label>
          </div>

          <div className="form-row">
            <span>{t("mcp.port")}</span>
            <input className="input-sm" value={portText} onChange={(e) => setPortText(e.target.value)} />
            <button className="btn btn-narrow" disabled={!st.enabled || Number(portText) === st.port} onClick={() => call("mcp_set_port", { port: Number(portText) })}>
              {t("mcp.apply")}
            </button>
          </div>

          <div className="form-row">
            <span>{t("mcp.token")}</span>
            <input readOnly value={showToken ? st.token : "•".repeat(Math.min(st.token.length, 24))} />
            <button className="btn btn-narrow" onClick={() => setShowToken((v) => !v)}>
              {showToken ? t("mcp.hide") : t("mcp.show")}
            </button>
            <button className="btn btn-narrow" onClick={() => call("mcp_rotate_token")} title={t("mcp.rotateHint")}>
              <IconRefresh {...ink(15)} />
            </button>
          </div>

          <div className="mcp-status">
            {st.running ? (
              <span>{t("mcp.running", { url: st.url })}</span>
            ) : (
              <span className="hint">{t("mcp.stopped")}</span>
            )}
            <button className="btn btn-narrow" onClick={() => copy(snippet(st))} title={t("mcp.copy")}>
              <IconCopy {...ink(15)} />
            </button>
            {copied && <span className="hint">{t("mcp.copied")}</span>}
          </div>
          {st.error && <div className="mcp-error">{t("mcp.error", { msg: st.error })}</div>}
          {err && <div className="mcp-error">{err}</div>}

          <div className="mcp-agents-title">{t("mcp.agents")}</div>
          {st.agents.length === 0 ? (
            <div className="hint">{t("mcp.noAgents")}</div>
          ) : (
            <div className="mcp-list">
              {st.agents.map((a) => (
                <div key={a.terminalId} className="mcp-row">
                  <span className="mcp-host">{a.sessionName}</span>
                  <span className="mcp-cmd" title={a.lastCommand}>
                    {a.lastCommand || "—"}
                  </span>
                  <span className={a.closed ? "mcp-closed" : "mcp-live"}>
                    {a.closed ? t("mcp.agentClosed") : t("mcp.agentLive")}
                  </span>
                </div>
              ))}
            </div>
          )}
          <div className="hint">{t("mcp.file", { path: st.configFile })}</div>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" autoFocus onClick={onClose}>
            {t("common.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
