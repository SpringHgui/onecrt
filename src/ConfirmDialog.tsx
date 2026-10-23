import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";

interface Props {
  message: string;
  onCancel(): void;
  onConfirm(): void;
}

export default function ConfirmDialog({ message, onCancel, onConfirm }: Props) {
  const { t } = useI18n();
  return (
    <div
      className="dialog-overlay"
      onMouseDown={(e) => e.target === e.currentTarget && onCancel()}
      onKeyDown={(e) => {
        // 面板挂在 flexlayout 里时，Esc 不能顺带把外层 overlay 面板也收掉
        if (e.key === "Escape") {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="dialog">
        <div className="dialog-titlebar">
          <span>{t("confirm.title")}</span>
          <button className="dialog-close" onClick={onCancel}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <p className="pwd-note">{message}</p>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" autoFocus onClick={onConfirm}>
            {t("common.ok")}
          </button>
          <button className="btn" onClick={onCancel}>
            {t("common.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
