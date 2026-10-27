import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";

/// 一个可导入的来源。logo 用厂商官方标识原件，只等比放进统一瓦片，不改几何与配色
/// （来源与权利人见 NOTICE.md）。
export interface ImportSource {
  /// 后端命令名：弹原生文件框选该产品的配置文件并解析
  cmd: "import_securecrt" | "import_winscp";
  logo: string;
  name: string;
  hintKey: string;
}

export const IMPORT_SOURCES: ImportSource[] = [
  { cmd: "import_securecrt", logo: "/brand/securecrt.png", name: "SecureCRT", hintKey: "import.securecrt.hint" },
  { cmd: "import_winscp", logo: "/brand/winscp.png", name: "WinSCP", hintKey: "import.winscp.hint" },
];

interface Props {
  onPick(src: ImportSource): void;
  onCancel(): void;
}

export default function ImportDialog({ onPick, onCancel }: Props) {
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
          <span>{t("import.title")}</span>
          <button className="dialog-close" onClick={onCancel}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <p className="pwd-note">{t("import.note")}</p>
          <div className="import-srcs">
            {IMPORT_SOURCES.map((s, i) => (
              <button key={s.cmd} className="import-src" autoFocus={i === 0} onClick={() => onPick(s)}>
                <img className="import-logo" src={s.logo} alt={`${s.name} logo`} width={40} height={40} />
                <span className="import-text">
                  <span className="import-name">{s.name}</span>
                  <span className="import-hint">{t(s.hintKey)}</span>
                </span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
