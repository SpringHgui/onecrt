import { useEffect, useRef, useState } from "react";
import { IconArrowDown, IconArrowUp } from "@tabler/icons-react";
import { fmtRate } from "./format";
import { ink } from "./icons";
import { pctOf, runningTransfer, useTransfers } from "./transfers";
import { useI18n } from "./i18n";
import { useMcp } from "./mcp";
import McpPeek from "./McpPeek";
import "./menu.css";

export type TabStatus = "connecting" | "ready" | "error" | "closed";

export interface StatusInfo {
  name: string;
  status: TabStatus;
  host: string;
  port: number;
  username: string;
  cols: number;
  rows: number;
}

interface Props {
  info: StatusInfo | null;
  tabCount: number;
  fontSize: number;
  onOpenMcp(): void;
}

export default function StatusBar({ info, tabCount, fontSize, onOpenMcp }: Props) {
  /// 传输进度常驻在这里：文件面板收起来或浮动到别处时也看得到
  const { t } = useI18n();
  const transfers = useTransfers();
  const rt = runningTransfer(transfers);
  const pct = rt ? pctOf(rt) : 0;

  /// MCP 一格常驻状态栏：平时五秒回一次，卡片浮起时跟弹窗一样降到两秒
  const [peek, setPeek] = useState<HTMLElement | null>(null);
  const mcp = useMcp(peek ? 2000 : 5000);
  const st = mcp.st;
  const closeTimer = useRef(0);
  const keepPeek = () => window.clearTimeout(closeTimer.current);
  const closePeek = () => {
    closeTimer.current = window.setTimeout(() => setPeek(null), 180);
  };
  useEffect(() => () => window.clearTimeout(closeTimer.current), []);

  const mcpDot = !st ? "closed" : st.running ? "ready" : st.enabled ? "error" : "closed";

  const STATUS_TEXT: Record<TabStatus, string> = {
    connecting: t("status.connecting"),
    ready: t("status.ready"),
    error: t("status.error"),
    closed: t("status.closed"),
  };

  return (
    <div className="statusbar">
      {info ? (
        <>
          <span className={`status-dot ${info.status}`} />
          <span className="sb-name">{info.name}</span>
          <span className="sb-sep" />
          <span>
            {info.username ? `${info.username}@` : ""}
            {info.host}:{info.port}
          </span>
          <span className="sb-sep" />
          <span>{STATUS_TEXT[info.status]}</span>
        </>
      ) : (
        <span>{t("status.noSession")}</span>
      )}

      {rt && (
        <>
          <span className="sb-sep" />
          <span className="sb-transfer" title={rt.name}>
            <span className="sb-dir">
              {rt.dir === "up" ? <IconArrowUp {...ink(12)} /> : <IconArrowDown {...ink(12)} />}
            </span>
            <span className="sb-tname">{rt.name}</span>
            <span className="sb-track">
              <i style={{ width: `${pct}%` }} />
            </span>
            <span className="sb-tpct">{pct}%</span>
            <span className="sb-trate">{fmtRate(rt.speed)}</span>
          </span>
        </>
      )}

      <span className="sb-fill" />
      {info && (
        <>
          <span>
            {info.cols} × {info.rows}
          </span>
          <span className="sb-sep" />
        </>
      )}
      <span>{t("status.tabs", { n: tabCount })}</span>
      <span className="sb-sep" />
      <span>{fontSize}px</span>
      <span className="sb-sep" />
      <button
        className={`sb-mcp ${mcpDot}`}
        title={st?.error ?? t("mcp.title")}
        onMouseEnter={(e) => {
          keepPeek();
          setPeek(e.currentTarget);
        }}
        onMouseLeave={closePeek}
        onClick={() => {
          keepPeek();
          setPeek(null);
          onOpenMcp();
        }}
      >
        <span className={`status-dot ${mcpDot}`} />
        <span>MCP</span>
        {st?.enabled && <span className="sb-mcp-port">:{st.port}</span>}
        {st?.running && st.agents.length > 0 && <span className="sb-mcp-n">·{st.agents.length}</span>}
      </button>
      {peek && (
        <McpPeek anchor={peek} mcp={mcp} onChanged={mcp.refresh} onEnter={keepPeek} onLeave={closePeek} />
      )}
    </div>
  );
}
