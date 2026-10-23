import {
  IconArrowsTransferUpDown,
  IconBolt,
  IconClipboard,
  IconCopy,
  IconRepeat,
  IconKeyboard,
  IconPrinter,
  IconRefresh,
  IconSettings,
  IconTool,
} from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";
import "./menu.css";

export interface ToolbarActions {
  /// 用活动标签的参数打开 Quick Connect，没有活动标签时是空白新建
  quickConnect(): void;
  cycleTab(): void;
  reconnect(): void;
  copy(): void;
  paste(): void;
  sessionOptions(): void;
  /** 开合右侧文件面板（SFTP） */
  fileTransfer(): void;
  filePanelOpen: boolean;
  tabCount: number;
}

export default function Toolbar(a: ToolbarActions) {
  const { t } = useI18n();
  const dead = a.tabCount === 0;
  return (
    <div className="toolbar">
      <button title={t("tb.newConnect")} onClick={a.quickConnect}>
        <IconBolt {...ink(17)} />
      </button>
      <button title={t("tb.nextTab")} disabled={a.tabCount < 2} onClick={a.cycleTab}>
        <IconRepeat {...ink(17)} />
      </button>
      <button title={t("tb.reconnect")} disabled={dead} onClick={a.reconnect}>
        <IconRefresh {...ink(17)} />
      </button>
      <span className="tb-sep" />
      <button title={t("tb.copy")} disabled={dead} onClick={a.copy}>
        <IconCopy {...ink(17)} />
      </button>
      <button title={t("tb.paste")} disabled={dead} onClick={a.paste}>
        <IconClipboard {...ink(17)} />
      </button>
      <button title={t("tb.print")} disabled>
        <IconPrinter {...ink(17)} />
      </button>
      <span className="tb-sep" />
      <button
        title={t("tb.fileTransfer")}
        aria-pressed={a.filePanelOpen}
        className={a.filePanelOpen ? "on" : undefined}
        disabled={dead}
        onClick={a.fileTransfer}
      >
        <IconArrowsTransferUpDown {...ink(17)} />
      </button>
      <span className="tb-sep" />
      <button title={t("tb.sessionOptions")} disabled={dead} onClick={a.sessionOptions}>
        <IconSettings {...ink(17)} />
      </button>
      <button title={t("tb.keyMapping")} disabled>
        <IconKeyboard {...ink(17)} />
      </button>
      <button title={t("tb.portForward")} disabled>
        <IconTool {...ink(17)} />
      </button>
    </div>
  );
}
