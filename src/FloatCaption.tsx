import { IconLayoutBoardSplit, IconX } from "@tabler/icons-react";
import { useLayoutEffect, useState } from "react";
import { createPortal } from "react-dom";
import { ink } from "./icons";
import { useI18n } from "./i18n";

interface FloatInfo {
  header: HTMLElement;
  tabs: string[];
}

const PREFIX = "flexlayout-tabbutton-";

/// VS 的浮窗顶栏右侧是"回停靠 / 关闭"一排按钮，库的 float header 只有抓手和标题，
/// 只能把按钮 portal 进去。浮窗是库渲染的 DOM，挂载时机不归我们管，
/// 用 MutationObserver 盯着 DOM 增删重扫，比跟着 action 猜时机可靠。
/// 浮窗里装着哪些标签从它底部那条标签条读（按钮 id 带节点 id）。
export default function FloatCaptionControls({
  onDockBack,
  onClose,
}: {
  onDockBack(tabId: string): void;
  onClose(tabIds: string[]): void;
}) {
  const [floats, setFloats] = useState<FloatInfo[]>([]);
  const { t } = useI18n();
  useLayoutEffect(() => {
    const scan = () => {
      const list: FloatInfo[] = [];
      for (const w of document.querySelectorAll(".flexlayout__float_window")) {
        const header = w.querySelector(".flexlayout__float_window_header");
        const tabs = [...w.querySelectorAll(`[id^=${PREFIX}]`)].map((b) => b.id.slice(PREFIX.length));
        if (header instanceof HTMLElement && tabs.length) list.push({ header, tabs });
      }
      setFloats(list);
    };
    scan();
    const mo = new MutationObserver(scan);
    mo.observe(document.body, { childList: true, subtree: true });
    return () => mo.disconnect();
  }, []);
  return floats
    .filter((f) => f.header.isConnected)
    .map((f) =>
      createPortal(
        // 顶栏的 pointerdown 是库搬窗口用的，按按钮时不能顺带把浮窗拖走
        <span className="float-caption-ctl" onPointerDown={(e) => e.stopPropagation()}>
          <button
            className="panel-pin panel-dockback"
            title={t("float.dockBack")}
            aria-label={t("float.dockBack")}
            onClick={() => onDockBack(f.tabs[0])}
          >
            <IconLayoutBoardSplit {...ink(14)} />
          </button>
          <button className="panel-pin" title={t("common.close")} aria-label={t("common.close")} onClick={() => onClose(f.tabs)}>
            <IconX {...ink(14)} />
          </button>
        </span>,
        f.header,
      ),
    );
}
