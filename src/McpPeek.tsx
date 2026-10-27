import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { IconCopy } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";
import { snippet, snippetMasked, useMcp } from "./mcp";
import "./menu.css";

interface Props {
  /// 状态栏那一格：卡片贴着它上沿展开
  anchor: HTMLElement;
  mcp: ReturnType<typeof useMcp>;
  /// 卡片里的操作会改状态，完成后让状态栏立刻回读一次
  onChanged(): void;
  /// 指针进/出卡片：由状态栏那格掌握延时，避免指针穿过 6px 缝隙时卡片先关掉
  onEnter(): void;
  onLeave(): void;
}

/// 状态栏 MCP 格的悬浮卡片：只读信息 + 复制 + 开关与改端口，全部走弹窗同一套命令。
/// 令牌按掩码显示，复制出去的是带真令牌的配置。
export default function McpPeek({ anchor, mcp, onChanged, onEnter, onLeave }: Props) {
  const { t } = useI18n();
  const { st, err, setErr, call } = mcp;
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: 0, y: 0 });
  const [maxH, setMaxH] = useState(0);
  const [copied, setCopied] = useState(false);
  const [portText, setPortText] = useState("");

  useEffect(() => {
    if (st) setPortText(String(st.port));
  }, [st?.port]);

  // 首帧按左上角落位，量完尺寸再挪进屏幕：横向不超出视口，纵向不许盖住状态栏那一格
  // （盖住了指针点格子会点进卡片，弹窗就打不开了），窗口太矮时卡片自己出滚动条。
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = anchor.getBoundingClientRect();
    const avail = Math.max(160, r.top - 10);
    const { width, height } = el.getBoundingClientRect();
    setMaxH(avail);
    setPos({
      x: Math.max(4, Math.min(r.left, window.innerWidth - width - 4)),
      y: r.top - Math.min(height, avail) - 6,
    });
  }, [anchor, st]);

  const copy = () => {
    if (!st) return;
    // WebView2 里 navigator.clipboard 要弹权限框、窗口失焦还会失败，一律走原生剪贴板
    invoke("clipboard_write", { text: snippet(st) }).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => setErr(t("mcp.copyFailed"))
    );
  };

  const run = (cmd: string, args?: Record<string, unknown>) => {
    call(cmd, args);
    onChanged();
  };

  const state = !st ? "" : st.running ? t("mcp.stateRunning") : st.enabled ? t("mcp.stopped") : t("mcp.stateDisabled");

  return createPortal(
    <div
      ref={ref}
      className="mcp-peek"
      style={{ left: pos.x, top: pos.y, maxHeight: maxH || undefined }}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      {st && (
        <>
          <div className="peek-head">
            <span className={`status-dot ${st.running ? "ready" : st.enabled ? "error" : "closed"}`} />
            <b>{t("mcp.title")}</b>
            <span className="peek-state">{state}</span>
          </div>

          <div className="peek-row">
            <span className="peek-label">{t("mcp.url")}</span>
            <code className="peek-val">{st.url}</code>
          </div>
          <div className="peek-note peek-file" title={st.configFile}>
            {t("mcp.file", { path: st.configFile })}
          </div>

          <div className="peek-row">
            <label className="peek-check">
              <input type="checkbox" checked={st.enabled} onChange={(e) => run("mcp_set_enabled", { enabled: e.target.checked })} />
              {t("mcp.enable")}
            </label>
          </div>
          <div className="peek-row">
            <span className="peek-label">{t("mcp.port")}</span>
            <input className="peek-port" value={portText} onChange={(e) => setPortText(e.target.value)} />
            <button className="btn btn-narrow" disabled={!st.enabled || Number(portText) === st.port} onClick={() => run("mcp_set_port", { port: Number(portText) })}>
              {t("mcp.apply")}
            </button>
          </div>

          <div className="peek-snippet">
            <pre>{snippetMasked(st)}</pre>
            <button className="btn btn-narrow peek-copy" onClick={copy} title={t("mcp.copyHint")}>
              <IconCopy {...ink(15)} />
            </button>
          </div>
          {copied && <div className="peek-note">{t("mcp.copied")}</div>}

          <div className="peek-agents">
            {t("mcp.agents")}
            <span className="peek-n">{st.agents.length}</span>
          </div>
          {st.agents.length === 0 ? (
            <div className="peek-note">{t("mcp.noAgents")}</div>
          ) : (
            <div className="peek-list">
              {st.agents.map((a) => (
                <div key={a.terminalId} className="peek-agent">
                  <span className="mcp-host">{a.sessionName}</span>
                  <span className="mcp-cmd" title={a.lastCommand}>
                    {a.lastCommand || "—"}
                  </span>
                  <span className={a.closed ? "mcp-closed" : "mcp-live"}>{a.closed ? t("mcp.agentClosed") : t("mcp.agentLive")}</span>
                </div>
              ))}
            </div>
          )}

          {(st.error || err) && <div className="peek-err">{st.error ? t("mcp.error", { msg: st.error }) : err}</div>}
        </>
      )}
    </div>,
    document.body
  );
}
