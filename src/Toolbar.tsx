import {
  IconArrowsTransferUpDown,
  IconBolt,
  IconClipboard,
  IconCopy,
  IconRepeat,
  IconDeviceDesktopPlus,
  IconKeyboard,
  IconPrinter,
  IconRefresh,
  IconSettings,
  IconTool,
} from "@tabler/icons-react";
import { ink } from "./icons";
import "./menu.css";

export interface ToolbarActions {
  newSession(): void;
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
  const dead = a.tabCount === 0;
  return (
    <div className="toolbar">
      <button title="新建会话" onClick={a.newSession}>
        <IconDeviceDesktopPlus {...ink(17)} />
      </button>
      <button title="快速连接" onClick={a.quickConnect}>
        <IconBolt {...ink(17)} />
      </button>
      <button title="切换到下一个标签" disabled={a.tabCount < 2} onClick={a.cycleTab}>
        <IconRepeat {...ink(17)} />
      </button>
      <button title="重新连接" disabled={dead} onClick={a.reconnect}>
        <IconRefresh {...ink(17)} />
      </button>
      <span className="tb-sep" />
      <button title="复制" disabled={dead} onClick={a.copy}>
        <IconCopy {...ink(17)} />
      </button>
      <button title="粘贴" disabled={dead} onClick={a.paste}>
        <IconClipboard {...ink(17)} />
      </button>
      <button title="打印" disabled>
        <IconPrinter {...ink(17)} />
      </button>
      <span className="tb-sep" />
      <button
        title="文件传输 / SFTP"
        aria-pressed={a.filePanelOpen}
        className={a.filePanelOpen ? "on" : undefined}
        disabled={dead}
        onClick={a.fileTransfer}
      >
        <IconArrowsTransferUpDown {...ink(17)} />
      </button>
      <span className="tb-sep" />
      <button title="会话选项" disabled={dead} onClick={a.sessionOptions}>
        <IconSettings {...ink(17)} />
      </button>
      <button title="键盘映射" disabled>
        <IconKeyboard {...ink(17)} />
      </button>
      <button title="端口转发 / 隧道" disabled>
        <IconTool {...ink(17)} />
      </button>
    </div>
  );
}
