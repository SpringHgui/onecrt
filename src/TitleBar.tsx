import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import MenuBar, { MenuDef } from "./MenuBar";
import { useI18n } from "./i18n";
import "./menu.css";

interface Props {
  menus: MenuDef[];
  /// 视图菜单里的"菜单栏"开关：只决定菜单标题项显不显示，标题栏本体常驻（原生装饰已关闭）
  menuVisible: boolean;
  title: string;
}

const run = (p: Promise<unknown>) => p.catch(console.error);

/// 系统 caption 按钮的字形度量：10x10 画框、1px 描边，直角带极小圆角
const glyphProps = {
  width: 10,
  height: 10,
  viewBox: "0 0 10 10",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1,
} as const;

const MinGlyph = () => (
  <svg {...glyphProps}>
    <path d="M0 4.5H10" />
  </svg>
);

const MaxGlyph = () => (
  <svg {...glyphProps}>
    <rect x="0.5" y="0.5" width="9" height="9" rx="0.5" />
  </svg>
);

/// 还原：后方框只画露出的三段边，前方框压在左下；两者外框都必须在 0..10 内，否则被 svg 裁掉
const RestoreGlyph = () => (
  <svg {...glyphProps}>
    <path d="M3.5 3.5V0.5H9.5V6.5H7" />
    <rect x="0.5" y="3.5" width="6" height="6" rx="0.5" />
  </svg>
);

const CloseGlyph = () => (
  <svg {...glyphProps}>
    <path d="M0.5 0.5L9.5 9.5M9.5 0.5L0.5 9.5" />
  </svg>
);

/// 自定义标题栏：菜单 + 窗口标题 + 窗口控制按钮，取代原生标题栏
export default function TitleBar({ menus, menuVisible, title }: Props) {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const w = getCurrentWindow();
    const read = () => w.isMaximized().then(setMaximized).catch(() => {});
    read();
    let unlisten: (() => void) | undefined;
    let dead = false;
    // 双击拖拽区、Win+方向键、拖到屏幕顶都会改变最大化态，图标得跟着换
    w.onResized(read)
      .then((f) => (dead ? f?.() : (unlisten = f)))
      .catch(() => {});
    return () => {
      dead = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className="titlebar" data-tauri-drag-region>
      {menuVisible && <MenuBar menus={menus} />}
      <div className="tb-title" data-tauri-drag-region title={title}>
        {title}
      </div>
      <div className="tb-caption">
        <button className="tb-btn" title={t("titlebar.minimize")} aria-label={t("titlebar.minimize")} onClick={() => run(getCurrentWindow().minimize())}>
          <span className="tb-glyph">
            <MinGlyph />
          </span>
        </button>
        <button
          className="tb-btn"
          title={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
          aria-label={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
          onClick={() => run(getCurrentWindow().toggleMaximize())}
        >
          <span className="tb-glyph">{maximized ? <RestoreGlyph /> : <MaxGlyph />}</span>
        </button>
        <button
          className="tb-btn close"
          title={t("titlebar.close")}
          aria-label={t("titlebar.close")}
          onClick={() => run(getCurrentWindow().close())}
        >
          <span className="tb-glyph">
            <CloseGlyph />
          </span>
        </button>
      </div>
    </div>
  );
}
