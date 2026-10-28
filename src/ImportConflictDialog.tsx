import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";

/// 导入冲突时的三选对话框：
///   - 只新增（默认安全）：跳过 imported 中命中现有的会话
///   - 新增 + 覆盖口令（btn-primary 但不 autofocus）：命中项按 2A 只改 password
///   - 取消：整个导入放弃
/// 覆盖按钮不做键盘默认，避免误按 Enter 就把用户手改过的口令回退。
interface Props {
  newCount: number;
  conflictCount: number;
  onAppendOnly(): void;
  onOverwrite(): void;
  onCancel(): void;
}

export default function ImportConflictDialog({
  newCount,
  conflictCount,
  onAppendOnly,
  onOverwrite,
  onCancel,
}: Props) {
  const { t } = useI18n();
  return (
    <div
      className="dialog-overlay"
      onMouseDown={(e) => e.target === e.currentTarget && onCancel()}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="dialog">
        <div className="dialog-titlebar">
          <span>{t("import.conflict.title")}</span>
          <button className="dialog-close" onClick={onCancel}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <p className="pwd-note">
            {t("import.conflict.body", { n: newCount, m: conflictCount })}
          </p>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" autoFocus onClick={onAppendOnly}>
            {t("import.conflict.append")}
          </button>
          <button className="btn" onClick={onOverwrite}>
            {t("import.conflict.overwrite")}
          </button>
          <button className="btn" onClick={onCancel}>
            {t("common.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
