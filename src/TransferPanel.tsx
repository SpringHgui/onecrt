import { IconArrowDown, IconArrowUp, IconEraser, IconX } from "@tabler/icons-react";
import { fmtElapsed, fmtRate, fmtSize } from "./format";
import { ink } from "./icons";
import PanelPin from "./PanelPin";
import { clearFinished, pctOf, useTransfers } from "./transfers";
import { useI18n } from "./i18n";
import "./transfer.css";

interface Props {
  onClose?(): void;
  /// 顶栏右上角的图钉：钉住=常驻，取消=自动隐藏
  pinned?: boolean;
  onTogglePin?(): void;
  /// 顶栏作为拖拽把手（flexlayout 拖放），由 App 注入
  drag?: React.HTMLAttributes<HTMLDivElement>;
  /// 嵌在文件面板底部：窗口身份（标题/图钉/关闭）归文件面板，这里只留一条窄标题行，
  /// 列也收窄——460px 的边栏放不下九列
  embedded?: boolean;
}

export default function TransferPanel({ onClose, pinned, onTogglePin, drag, embedded }: Props) {
  const { t } = useI18n();
  const items = useTransfers();
  const finished = items.some((t) => t.status !== "running");

  const statusText = {
    running: t("tp.status.running"),
    done: t("tp.status.done"),
    error: t("tp.status.error"),
  } as const;

  return (
    <div className={`transferpanel${embedded ? " transferpanel--embedded" : ""}`}>
      <div className={`tp-bar${embedded ? " tp-bar--slim" : ""}`} {...(embedded ? {} : drag)}>
        <span className="tp-title">{t("tp.title")}</span>
        <span className="tp-spacer" />
        {!embedded && onTogglePin && <PanelPin pinned={!!pinned} onToggle={onTogglePin} />}
        <button title={t("tp.clearFinished")} disabled={!finished} onClick={clearFinished}>
          <IconEraser {...ink(16)} />
        </button>
        {!embedded && onClose && (
          <button title={t("tp.close")} onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        )}
      </div>

      <div className="tp-head">
        <span />
        <span>{t("tp.colFile")}</span>
        <span>{t("tp.colProgress")}</span>
        <span className="tp-num">%</span>
        {!embedded && <span className="tp-num">{t("tp.colSize")}</span>}
        {!embedded && <span className="tp-num">{t("tp.colRate")}</span>}
        {!embedded && <span className="tp-num">{t("tp.colElapsed")}</span>}
        <span>{t("tp.colStatus")}</span>
      </div>

      <div className="tp-list">
        {items.map((tr) => {
          const pct = pctOf(tr);
          const ms = (tr.endedAt || Date.now()) - tr.startedAt;
          const detail = `${tr.name}\n${tr.total ? `${fmtSize(tr.done)} / ${fmtSize(tr.total)}` : fmtSize(tr.done)} · ${
            tr.status === "running" ? fmtRate(tr.speed) : "—"
          } · ${fmtElapsed(ms)}${tr.error ? `\n${tr.error}` : ""}`;
          return (
            <div key={tr.id} className={`tp-row ${tr.status}`} title={detail}>
              <span className="tp-dir">
                {tr.dir === "up" ? <IconArrowUp {...ink(14)} /> : <IconArrowDown {...ink(14)} />}
              </span>
              <span className="tp-name">{tr.name}</span>
              <span className="tp-track">
                <i style={{ width: `${pct}%` }} />
              </span>
              <span className="tp-num">{pct}</span>
              {!embedded && <span className="tp-num">{tr.total ? `${fmtSize(tr.done)} / ${fmtSize(tr.total)}` : fmtSize(tr.done)}</span>}
              {!embedded && <span className="tp-num">{tr.status === "running" ? fmtRate(tr.speed) : "—"}</span>}
              {!embedded && <span className="tp-num">{fmtElapsed(ms)}</span>}
              <span className="tp-status">{statusText[tr.status]}</span>
            </div>
          );
        })}
        {items.length === 0 && <p className="tp-empty">{t("tp.empty")}</p>}
      </div>
    </div>
  );
}
