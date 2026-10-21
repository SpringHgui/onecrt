import { MenuDef } from "./MenuBar";
import { MenuItem } from "./ContextMenu";
import { BASE_FONT, ViewKey, ViewState } from "./view";

export interface MenuCtx {
  view: ViewState;
  theme: "dark" | "light";
  fontSize: number;
  fullScreen: boolean;
  alwaysOnTop: boolean;
  /// 当前是否有活动标签 / 活动标签是否已连上
  hasTab: boolean;
  tabReady: boolean;
  toggleView(k: ViewKey): void;
  resetLayout(): void;
  toggleTheme(): void;
  newSession(): void;
  newFolder(): void;
  closeTab(): void;
  quit(): void;
  reconnect(): void;
  disconnect(): void;
  sendInterrupt(): void;
  clearScreen(): void;
  zoomIn(): void;
  zoomOut(): void;
  zoomReset(): void;
  toggleFullScreen(): void;
  toggleAlwaysOnTop(): void;
  about(): void;
  /// 终端选区复制/粘贴依赖 Terminal 暴露的 API，未接上时传 undefined，菜单项自动置灰
  copyTerminal?(): void;
  pasteTerminal?(): void;
  selectAllTerminal?(): void;
}

const viewToggle = (c: MenuCtx, key: ViewKey, label: string, accel?: string): MenuItem => ({
  label,
  accel,
  checked: c.view[key],
  onClick: () => c.toggleView(key),
});

export function buildMenus(c: MenuCtx): MenuDef[] {
  return [
    {
      label: "文件",
      items: [
        { label: "新建会话…", onClick: c.newSession },
        { label: "新建文件夹…", onClick: c.newFolder },
        {},
        { label: "关闭当前标签", accel: "Ctrl+W", disabled: !c.hasTab, onClick: c.closeTab },
        {},
        { label: "退出", onClick: c.quit },
      ],
    },
    {
      label: "编辑",
      items: [
        { label: "复制", accel: "Ctrl+C", disabled: !c.copyTerminal || !c.hasTab, onClick: c.copyTerminal },
        { label: "粘贴", accel: "Ctrl+V", disabled: !c.pasteTerminal || !c.hasTab, onClick: c.pasteTerminal },
        { label: "全选", accel: "Ctrl+Shift+A", disabled: !c.selectAllTerminal || !c.hasTab, onClick: c.selectAllTerminal },
      ],
    },
    {
      label: "终端",
      items: [
        { label: "重新连接", disabled: !c.hasTab, onClick: c.reconnect },
        { label: "断开连接", disabled: !c.hasTab, onClick: c.disconnect },
        {},
        { label: "发送 Ctrl+C", disabled: !c.tabReady, onClick: c.sendInterrupt },
        { label: "清屏", disabled: !c.tabReady, onClick: c.clearScreen },
      ],
    },
    {
      label: "视图",
      items: [
        viewToggle(c, "menuBar", "菜单栏", "Alt"),
        viewToggle(c, "toolbar", "工具栏"),
        viewToggle(c, "sessionTabs", "会话标签栏"),
        viewToggle(c, "statusBar", "状态栏"),
        {},
        viewToggle(c, "sessionManager", "会话管理器"),
        viewToggle(c, "cmdManager", "命令管理器"),
        {
          label: "文件面板 (SFTP)",
          checked: c.view.filePanel,
          disabled: !c.hasTab,
          onClick: () => c.toggleView("filePanel"),
        },
        {},
        { label: "重置窗口布局", onClick: c.resetLayout },
        {},
        { label: "放大", accel: "Ctrl+=", onClick: c.zoomIn },
        { label: "缩小", accel: "Ctrl+-", onClick: c.zoomOut },
        { label: "重置缩放", accel: "Ctrl+0", disabled: c.fontSize === BASE_FONT, onClick: c.zoomReset },
        {},
        { label: "深色主题", checked: c.theme === "dark", onClick: c.toggleTheme },
        { label: "全屏", accel: "F11", checked: c.fullScreen, onClick: c.toggleFullScreen },
        { label: "总在最前", checked: c.alwaysOnTop, onClick: c.toggleAlwaysOnTop },
      ],
    },
    {
      label: "帮助",
      items: [{ label: "关于 ShellX…", onClick: c.about }],
    },
  ];
}
