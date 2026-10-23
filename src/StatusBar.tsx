import { IconArrowDown, IconArrowUp } from "@tabler/icons-react";
import { fmtRate } from "./format";
import { ink } from "./icons";
import { pctOf, runningTransfer, useTransfers } from "./transfers";
import { useI18n } from "./i18n";
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
}

export default function StatusBar({ info, tabCount, fontSize }: Props) {
  /// 传输进度常驻在这里：文件面板收起来或浮动到别处时也看得到
  const { t } = useI18n();
  const transfers = useTransfers();
  const rt = runningTransfer(transfers);
  const pct = rt ? pctOf(rt) : 0;

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
    </div>
  );
}
