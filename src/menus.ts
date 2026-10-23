import { MenuDef } from "./MenuBar";
import { MenuItem } from "./ContextMenu";
import { BASE_FONT, ViewKey, ViewState } from "./view";
import { Locale, t } from "./i18n";

export interface MenuCtx {
  view: ViewState;
  theme: "dark" | "light";
  fontSize: number;
  fullScreen: boolean;
  alwaysOnTop: boolean;
  /// 当前是否有活动标签 / 活动标签是否已连上
  hasTab: boolean;
  tabReady: boolean;
  locale: Locale;
  setLocale(l: Locale): void;
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
      label: t("menu.file"),
      items: [
        { label: t("menu.file.newSession"), onClick: c.newSession },
        { label: t("menu.file.newFolder"), onClick: c.newFolder },
        {},
        { label: t("menu.file.closeTab"), accel: "Ctrl+W", disabled: !c.hasTab, onClick: c.closeTab },
        {},
        { label: t("menu.file.quit"), onClick: c.quit },
      ],
    },
    {
      label: t("menu.edit"),
      items: [
        { label: t("menu.edit.copy"), accel: "Ctrl+C", disabled: !c.copyTerminal || !c.hasTab, onClick: c.copyTerminal },
        { label: t("menu.edit.paste"), accel: "Ctrl+V", disabled: !c.pasteTerminal || !c.hasTab, onClick: c.pasteTerminal },
        { label: t("menu.edit.selectAll"), accel: "Ctrl+Shift+A", disabled: !c.selectAllTerminal || !c.hasTab, onClick: c.selectAllTerminal },
      ],
    },
    {
      label: t("menu.terminal"),
      items: [
        { label: t("menu.terminal.reconnect"), disabled: !c.hasTab, onClick: c.reconnect },
        { label: t("menu.terminal.disconnect"), disabled: !c.hasTab, onClick: c.disconnect },
        {},
        { label: t("menu.terminal.sendCtrlC"), disabled: !c.tabReady, onClick: c.sendInterrupt },
        { label: t("menu.terminal.clear"), disabled: !c.tabReady, onClick: c.clearScreen },
      ],
    },
    {
      label: t("menu.view"),
      items: [
        viewToggle(c, "menuBar", t("menu.view.menuBar"), "Alt"),
        viewToggle(c, "toolbar", t("menu.view.toolbar")),
        viewToggle(c, "sessionTabs", t("menu.view.sessionTabs")),
        viewToggle(c, "statusBar", t("menu.view.statusBar")),
        {},
        viewToggle(c, "sessionManager", t("menu.view.sessionManager")),
        viewToggle(c, "cmdManager", t("menu.view.cmdManager")),
        {
          label: t("menu.view.filePanel"),
          checked: c.view.filePanel,
          disabled: !c.hasTab,
          onClick: () => c.toggleView("filePanel"),
        },
        viewToggle(c, "transfers", t("menu.view.transfers")),
        {},
        {
          label: t("menu.view.language"),
          items: [
            { label: "中文", checked: c.locale === "zh", onClick: () => c.setLocale("zh") },
            { label: "English", checked: c.locale === "en", onClick: () => c.setLocale("en") },
          ],
        },
        { label: t("menu.view.resetLayout"), onClick: c.resetLayout },
        {},
        { label: t("menu.view.zoomIn"), accel: "Ctrl+=", onClick: c.zoomIn },
        { label: t("menu.view.zoomOut"), accel: "Ctrl+-", onClick: c.zoomOut },
        { label: t("menu.view.zoomReset"), accel: "Ctrl+0", disabled: c.fontSize === BASE_FONT, onClick: c.zoomReset },
        {},
        { label: t("menu.view.darkTheme"), checked: c.theme === "dark", onClick: c.toggleTheme },
        { label: t("menu.view.fullscreen"), accel: "F11", checked: c.fullScreen, onClick: c.toggleFullScreen },
        { label: t("menu.view.alwaysOnTop"), checked: c.alwaysOnTop, onClick: c.toggleAlwaysOnTop },
      ],
    },
    {
      label: t("menu.help"),
      items: [{ label: t("menu.help.about"), onClick: c.about }],
    },
  ];
}
