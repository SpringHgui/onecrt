import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ViewKey =
  | "menuBar"
  | "toolbar"
  | "sessionTabs"
  | "statusBar"
  | "sessionManager"
  | "cmdManager"
  | "filePanel";

export type ViewState = Record<ViewKey, boolean>;

const VIEW_STORE = "shellx.view";
const ZOOM_STORE = "shellx.zoom";

export const BASE_FONT = 14;
export const MIN_FONT = 9;
export const MAX_FONT = 32;

export const DEFAULT_VIEW: ViewState = {
  menuBar: true,
  toolbar: true,
  sessionTabs: true,
  statusBar: true,
  sessionManager: true,
  cmdManager: true,
  filePanel: false,
};

function loadView(): ViewState {
  try {
    const raw = localStorage.getItem(VIEW_STORE);
    if (raw) return { ...DEFAULT_VIEW, ...(JSON.parse(raw) as Partial<ViewState>) };
  } catch (e) {
    console.error(VIEW_STORE, e);
  }
  return DEFAULT_VIEW;
}

/// 各面板的显示/隐藏，改动即时写入 localStorage
export function useView() {
  const [view, setView] = useState<ViewState>(loadView);
  useEffect(() => {
    localStorage.setItem(VIEW_STORE, JSON.stringify(view));
  }, [view]);
  const toggleView = useCallback((k: ViewKey) => setView((v) => ({ ...v, [k]: !v[k] })), []);
  return { view, toggleView, setView };
}

function loadFont(): number {
  const n = Number(localStorage.getItem(ZOOM_STORE));
  return Number.isFinite(n) && n >= MIN_FONT && n <= MAX_FONT ? n : BASE_FONT;
}

/// 终端字号。真正生效要把它传给 <Terminal fontSize>，见接线说明
export function useZoom() {
  const [fontSize, setFontSize] = useState(loadFont);
  useEffect(() => {
    localStorage.setItem(ZOOM_STORE, String(fontSize));
  }, [fontSize]);
  const zoomIn = useCallback(() => setFontSize((n) => Math.min(MAX_FONT, n + 1)), []);
  const zoomOut = useCallback(() => setFontSize((n) => Math.max(MIN_FONT, n - 1)), []);
  const zoomReset = useCallback(() => setFontSize(BASE_FONT), []);
  return { fontSize, zoomIn, zoomOut, zoomReset };
}

/// 全屏与总在最前：走原生窗口 API，取不到状态时退回本地记录的值
export function useWindowToggles() {
  const [fullScreen, setFullScreen] = useState(false);
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);

  const toggleFullScreen = useCallback(async () => {
    const w = getCurrentWindow();
    const cur = await w.isFullscreen().catch(() => fullScreen);
    const next = !cur;
    try {
      await w.setFullscreen(next);
      setFullScreen(next);
    } catch (e) {
      console.error("setFullscreen", e);
    }
  }, [fullScreen]);

  const toggleAlwaysOnTop = useCallback(async () => {
    const w = getCurrentWindow();
    const cur = await w.isAlwaysOnTop().catch(() => alwaysOnTop);
    const next = !cur;
    try {
      await w.setAlwaysOnTop(next);
      setAlwaysOnTop(next);
    } catch (e) {
      console.error("setAlwaysOnTop", e);
    }
  }, [alwaysOnTop]);

  return { fullScreen, toggleFullScreen, alwaysOnTop, toggleAlwaysOnTop };
}
