import { IconArrowDown, IconArrowUp, IconEraser, IconX } from "@tabler/icons-react";
import { fmtElapsed, fmtRate, fmtSize } from "./format";
import { ink } from "./icons";
import PanelPin from "./PanelPin";
import { clearFinished, pctOf, useTransfers } from "./transfers";
import { useI18n } from "./i18n";
import "./transfer.css";

interface Props {
  onClose(): void;
  /// 顶栏右上角的图钉：钉住=常驻，取消=自动隐藏
  pinned?: boolean;
  onTogglePin?(): void;
  /// 顶栏作为拖拽把手（flexlayout 拖放），由 App 注入
  drag?: React.HTMLAttributes<HTMLDivElement>;
}

export default function TransferPanel({ onClose, pinned, onTogglePin, drag }: Props) {
  const { t } = useI18n();
  const items = useTransfers();
  const finished = items.some((t) => t.status !== "running");

  const statusText = {
    running: t("tp.status.running"),
    done: t("tp.status.done"),
    error: t("tp.status.error"),
  } as const;

  return (
    <div className="transferpanel">
      <div className="tp-bar" {...drag}>
        <span className="tp-title">{t("tp.title")}</span>
        <span className="tp-spacer" />
        {onTogglePin && <PanelPin pinned={!!pinned} onToggle={onTogglePin} />}
        <button title={t("tp.clearFinished")} disabled={!finished} onClick={clearFinished}>
          <IconEraser {...ink(16)} />
        </button>
        <button title={t("tp.close")} onClick={onClose}>
          <IconX {...ink(16)} />
        </button>
      </div>

      <div className="tp-head">
        <span />
        <span>{t("tp.colFile")}</span>
        <span>{t("tp.colProgress")}</span>
        <span className="tp-num">%</span>
        <span className="tp-num">{t("tp.colSize")}</span>
        <span className="tp-num">{t("tp.colRate")}</span>
        <span className="tp-num">{t("tp.colElapsed")}</span>
        <span>{t("tp.colStatus")}</span>
      </div>

      <div className="tp-list">
        {items.map((tr) => {
          const pct = pctOf(tr);
          const ms = (tr.endedAt || Date.now()) - tr.startedAt;
          return (
            <div key={tr.id} className={`tp-row ${tr.status}`} title={tr.error || tr.name}>
              <span className="tp-dir">
                {tr.dir === "up" ? <IconArrowUp {...ink(14)} /> : <IconArrowDown {...ink(14)} />}
              </span>
              <span className="tp-name">{tr.name}</span>
              <span className="tp-track">
                <i style={{ width: `${pct}%` }} />
              </span>
              <span className="tp-num">{pct}</span>
              <span className="tp-num">{tr.total ? `${fmtSize(tr.done)} / ${fmtSize(tr.total)}` : fmtSize(tr.done)}</span>
              <span className="tp-num">{tr.status === "running" ? fmtRate(tr.speed) : "—"}</span>
              <span className="tp-num">{fmtElapsed(ms)}</span>
              <span className="tp-status">{statusText[tr.status]}</span>
            </div>
          );
        })}
        {items.length === 0 && <p className="tp-empty">{t("tp.empty")}</p>}
      </div>
    </div>
  );
}
