import { useEffect, useRef, useState } from "react";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import ContextMenu, { MenuItem } from "./ContextMenu";
import "./menu.css";

export interface MenuDef {
  label: string;
  items: MenuItem[];
}

interface Props {
  menus: MenuDef[];
}

/// 顶部菜单栏。下拉直接复用右键菜单的渲染（.ctx-menu），不再写一份菜单项样式
export default function MenuBar({ menus }: Props) {
  const [open, setOpen] = useState(-1);
  const barRef = useRef<HTMLDivElement>(null);
  // 记录 mousedown 时该标题是否已展开：ContextMenu 会先因"点击外部"关掉自己，
  // 若不在 click 阶段补救，再点同一个标题就会关掉变重开
  const wasOpen = useRef(false);

  useEffect(() => {
    if (open < 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight") setOpen((i) => (i + 1) % menus.length);
      else if (e.key === "ArrowLeft") setOpen((i) => (i - 1 + menus.length) % menus.length);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, menus.length]);

  const title = (i: number) => {
    const el = barRef.current?.querySelector<HTMLElement>(`[data-menu="${i}"]`);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.left, y: r.bottom + 2 };
  };

  const pos = open >= 0 ? title(open) : null;

  return (
    <>
      <div className="menubar" ref={barRef}>
        {menus.map((m, i) => (
          <div
            key={m.label}
            data-menu={i}
            className={`menubar-title${i === open ? " open" : ""}`}
            onMouseDown={() => (wasOpen.current = i === open)}
            onMouseEnter={() => open >= 0 && i !== open && setOpen(i)}
            onClick={() => setOpen(wasOpen.current ? -1 : i)}
          >
            {m.label}
          </div>
        ))}
      </div>
      {open >= 0 && pos && (
        <ContextMenu
          key={open}
          x={pos.x}
          y={pos.y}
          items={menus[open].items}
          onClose={() => setOpen(-1)}
        />
      )}
    </>
  );
}

export interface ShortcutHandlers {
  /// 菜单栏当前是否可见。Alt 只在它已经藏起来时把它唤回——
  /// 做成双向开关的话，用户 Alt+Tab 切回来就把菜单栏点没了
  menuBarVisible?: boolean;
  onMenuBar?(): void;
  onFullScreen?(): void;
  onCloseTab?(): void;
  onZoomIn?(): void;
  onZoomOut?(): void;
  onZoomReset?(): void;
}

/// 菜单栏里带 accel 标注的那几项的真实按键。挂在 App 层，菜单栏隐藏时也要能收到 Alt
export function useMenuShortcuts(h: ShortcutHandlers) {
  const ref = useRef(h);
  ref.current = h;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.repeat) return;
      const k = ref.current;
      switch (e.key) {
        case "F11":
          e.preventDefault();
          k.onFullScreen?.();
          break;
        case "Alt":
          if (k.menuBarVisible) return;
          e.preventDefault();
          k.onMenuBar?.();
          break;
        case "w":
        case "W":
          if (!e.ctrlKey) return;
          e.preventDefault();
          k.onCloseTab?.();
          break;
        case "=":
        case "+":
          if (!e.ctrlKey) return;
          e.preventDefault();
          k.onZoomIn?.();
          break;
        case "-":
        case "_":
          if (!e.ctrlKey) return;
          e.preventDefault();
          k.onZoomOut?.();
          break;
        case "0":
          if (!e.ctrlKey) return;
          e.preventDefault();
          k.onZoomReset?.();
          break;
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);
}

export function AboutDialog({ version, onClose }: { version: string; onClose(): void }) {
  return (
    <div className="dialog-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog dialog-sm">
        <div className="dialog-titlebar">
          <span>关于 ShellX</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <div className="about-line">
            <b>ShellX</b> {version}
          </div>
          <div className="hint">
            跨平台 SSH 客户端：Tauri 2 + Rust（russh）+ React + xterm.js。
          </div>
          <div className="hint">会话与命令保存在 %APPDATA%\shellx\，口令明文存储，仅面向内网运维。</div>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" onClick={onClose}>
            确定
          </button>
        </div>
      </div>
    </div>
  );
}
