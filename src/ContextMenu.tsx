import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { IconChevronRight } from "@tabler/icons-react";
import { ink } from "./icons";

export interface MenuItem {
  /// 缺省表示一条分隔线
  label?: string;
  accel?: string;
  disabled?: boolean;
  checked?: boolean;
  /// 只有箭头、暂时没有内容的占位子菜单
  hasSubmenu?: boolean;
  /// 子菜单内容；只支持一层，对应 "Send Command to" 的形态
  items?: MenuItem[];
  onClick?(): void;
}

interface Props {
  x: number;
  y: number;
  items: MenuItem[];
  onClose(): void;
}

const selectable = (it: MenuItem) => it.label !== undefined && !it.disabled;
const hasSub = (it: MenuItem) => !!it.items?.length;

/// 子菜单贴着自己这一层渲染，但两张菜单都要算出屏幕内的位置
function clamp(x: number, y: number, w: number, h: number) {
  return {
    x: Math.max(4, Math.min(x, window.innerWidth - w - 4)),
    y: Math.max(4, Math.min(y, window.innerHeight - h - 4)),
  };
}

export default function ContextMenu({ x, y, items, onClose }: Props) {
  const layerRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const subRef = useRef<HTMLDivElement>(null);
  const rows = useRef<(HTMLDivElement | null)[]>([]);
  const [pos, setPos] = useState({ x, y });
  const [hot, setHot] = useState(-1);
  const [sub, setSub] = useState<null | { i: number; x: number; y: number }>(null);
  const [subHot, setSubHot] = useState(-1);

  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos(clamp(x, y, width, height));
  }, [x, y]);

  const subItems = sub ? items[sub.i]?.items : undefined;
  const subI = sub?.i ?? -1;

  // 首帧按未夹取的位置渲染，量完尺寸再挪进屏幕，避免子菜单超出右/下边界
  useLayoutEffect(() => {
    const el = subRef.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setSub((s) => {
      if (!s) return s;
      const next = clamp(s.x, s.y, width, height);
      return next.x === s.x && next.y === s.y ? s : { ...s, ...next };
    });
  }, [subI]);

  const openSub = (i: number) => {
    const el = rows.current[i];
    if (!el) return;
    const r = el.getBoundingClientRect();
    setSub({ i, x: r.right - 4, y: r.top - 6 });
    setSubHot(-1);
  };

  useEffect(() => {
    const step = (list: MenuItem[], cur: number, dir: number, set: (i: number) => void) => {
      let i = cur;
      for (let k = 0; k < list.length; k++) {
        i = (i + dir + list.length) % list.length;
        if (selectable(list[i])) return set(i);
      }
    };

    const activate = (list: MenuItem[], i: number) => {
      const it = list[i];
      if (!it || it.disabled || hasSub(it)) return;
      onClose();
      it.onClick?.();
    };

    const onDown = (e: MouseEvent) => {
      if (!layerRef.current?.contains(e.target as globalThis.Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (subItems) setSub(null);
        else onClose();
      } else if (e.key === "ArrowRight" && !subItems) {
        if (hot >= 0 && hasSub(items[hot])) {
          e.preventDefault();
          openSub(hot);
        }
      } else if (e.key === "ArrowLeft" && subItems) {
        e.preventDefault();
        setSub(null);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        if (subItems) step(subItems, subHot, 1, setSubHot);
        else step(items, hot, 1, setHot);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        if (subItems) step(subItems, subHot, -1, setSubHot);
        else step(items, hot, -1, setHot);
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (subItems) activate(subItems, subHot);
        else activate(items, hot);
      }
    };
    document.addEventListener("mousedown", onDown, true);
    document.addEventListener("keydown", onKey, true);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      document.removeEventListener("keydown", onKey, true);
      window.removeEventListener("resize", onClose);
    };
  }, [items, subItems, hot, subHot, sub, onClose]);

  const renderList = (
    list: MenuItem[],
    cur: number,
    setCur: (i: number) => void,
    onEnter?: (it: MenuItem, i: number, el: HTMLDivElement) => void,
  ) =>
    list.map((it, i) =>
      it.label === undefined ? (
        <div key={i} className="ctx-sep" />
      ) : (
        <div
          key={i}
          ref={
            onEnter
              ? (el) => {
                  rows.current[i] = el;
                }
              : undefined
          }
          className={`ctx-item${it.disabled ? " disabled" : ""}${i === cur ? " hot" : ""}`}
          onMouseEnter={(e) => {
            if (!selectable(it)) return;
            setCur(i);
            onEnter?.(it, i, e.currentTarget);
          }}
          onClick={() => {
            if (it.disabled) return;
            if (hasSub(it)) return;
            onClose();
            it.onClick?.();
          }}
        >
          <span className={`ctx-check${it.checked ? " on" : ""}`} />
          <span className="ctx-label">{it.label}</span>
          <span className="ctx-accel">{it.accel ?? ""}</span>
          <span className="ctx-sub">{(it.hasSubmenu || hasSub(it)) && <IconChevronRight {...ink(14)} />}</span>
        </div>
      ),
    );

  return createPortal(
    <div ref={layerRef} className="ctx-layer">
      <div
        ref={menuRef}
        className="ctx-menu"
        style={{ left: pos.x, top: pos.y }}
        onContextMenu={(e) => e.preventDefault()}
      >
        {renderList(items, hot, setHot, (it, i) => (hasSub(it) ? openSub(i) : setSub(null)))}
      </div>
      {sub && subItems && (
        <div
          ref={subRef}
          className="ctx-menu ctx-submenu"
          style={{ left: sub.x, top: sub.y }}
          onContextMenu={(e) => e.preventDefault()}
        >
          {renderList(subItems, subHot, setSubHot)}
        </div>
      )}
    </div>,
    document.body,
  );
}
