import { forwardRef } from "react";

export type DockZone = "center" | "top" | "right" | "bottom" | "left";

export type CompassTarget = { x: number; y: number; w: number; h: number; zone: DockZone };

/// flexlayout 的 DockLocation.getLocation 标了 @internal，导不出来，这里照同一份公式复算：
/// 中间 50%×50% 归 center，其余按对角线四分。必须逐字一致——罗盘指错方向比没有罗盘更糟。
export function zoneAt(r: { x: number; y: number; width: number; height: number }, px: number, py: number): DockZone {
  if (!(r.width > 0) || !(r.height > 0)) return "center";
  const x = (px - r.x) / r.width;
  const y = (py - r.y) / r.height;
  if (x >= 0.25 && x < 0.75 && y >= 0.25 && y < 0.75) return "center";
  const bl = y >= x;
  const br = y >= 1 - x;
  if (bl) return br ? "bottom" : "left";
  return br ? "right" : "top";
}

/// 指针底下最靠上的那个标签区内容矩形（库量给 contentRect 的正是这个元素）。
/// 不能用 elementFromPoint：拖动期间库会盖一层 .flexlayout__layout_overlay（z-index 1000，
/// 且没有 pointer-events:none）挡在最上面，只会拿到它。改看整条命中栈、跳过这层遮罩。
/// 好处是展开的 overlay 面板本来就排在栈前面，它的 closest 找不到 tabset_content，
/// 罗盘就不出现——正好和库把落点判给面板的行为对上。
export function compassTargetAt(px: number, py: number): CompassTarget | null {
  const hit = document
    .elementsFromPoint(px, py)
    .find((e) => !e.classList.contains("flexlayout__layout_overlay"));
  const el = hit?.closest(".flexlayout__tabset_content");
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) return null;
  return { x: r.x, y: r.y, w: r.width, h: r.height, zone: zoneAt(r, px, py) };
}

/// 罗盘本身是静态骨架，拖动时由 App 直接改 style 与 data-zone（dragover 每秒几十次，
/// 走 setState 会连带重渲染整棵 App，终端在里面）。
const DockCompass = forwardRef<HTMLDivElement>(function DockCompass(_props, ref) {
  return (
    <div className="dock-compass" ref={ref} hidden aria-hidden="true">
      <span className="dock-compass-dial">
        <span className="dock-compass-arm" data-arm="top" />
        <span className="dock-compass-arm" data-arm="right" />
        <span className="dock-compass-arm" data-arm="bottom" />
        <span className="dock-compass-arm" data-arm="left" />
        <span className="dock-compass-hub" data-arm="center" />
      </span>
    </div>
  );
});

export default DockCompass;
