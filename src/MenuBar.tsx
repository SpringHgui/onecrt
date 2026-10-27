import { useEffect, useRef, useState } from "react";
import { IconExternalLink, IconX } from "@tabler/icons-react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ink } from "./icons";
import ContextMenu, { MenuItem } from "./ContextMenu";
import { useI18n } from "./i18n";
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

/// 仓库地址：本地工作副本的目录名与远端不同，关于页的链接一律按远端写
const REPO = "https://github.com/SpringHgui/onecrt";

/// 关于页的外链。openUrl 由 opener 插件按默认浏览器打开，http/https 已在 opener:default 作用域里
const ABOUT_LINKS = [
  { key: "about.link.home", url: REPO },
  { key: "about.link.releases", url: `${REPO}/releases` },
  { key: "about.link.issues", url: `${REPO}/issues/new` },
  { key: "about.link.notice", url: `${REPO}/blob/master/NOTICE.md` },
];

/// 直接依赖，地址取自各自 package.json 的 homepage / repository
const ABOUT_CREDITS = [
  { name: "Tauri 2", url: "https://tauri.app" },
  { name: "russh", url: "https://github.com/warp-tech/russh" },
  { name: "xterm.js", url: "https://github.com/xtermjs/xterm.js" },
  { name: "FlexLayout", url: "https://github.com/caplin/FlexLayout" },
  { name: "Tabler Icons", url: "https://tabler.io/icons" },
];

const OS_LABEL: Record<string, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };
const ARCH_LABEL: Record<string, string> = { x86_64: "x64", aarch64: "ARM64" };

interface AppInfo {
  os: string;
  arch: string;
  dataDir: string;
}

export function AboutDialog({ onClose }: { onClose(): void }) {
  const { t } = useI18n();
  /// 版本取运行时注入的值（唯一数据源是 tauri.conf.json），别再在代码里抄一份常量
  const [version, setVersion] = useState("");
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [copied, setCopied] = useState(false);
  const [upd, setUpd] = useState<"idle" | "checking" | "installing" | "latest" | "error">("idle");
  const [pending, setPending] = useState("");

  useEffect(() => {
    getVersion().then(setVersion, () => setVersion(""));
    invoke<AppInfo>("app_info").then(setInfo, () => setInfo(null));
  }, []);

  const envLine = info
    ? [OS_LABEL[info.os] ?? info.os, ARCH_LABEL[info.arch] ?? info.arch].filter(Boolean).join(" · ")
    : "";

  /// 提 issue 时把环境一次贴走，省得来回问「你什么系统、哪个版本」
  const copyInfo = () => {
    const text = [
      `OneCRT ${version}`,
      envLine,
      info?.dataDir ? `${t("about.dataDir")}: ${info.dataDir}` : "",
    ]
      .filter(Boolean)
      .join("\n");
    invoke("clipboard_write", { text }).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1600);
      },
      (e) => console.error("copy", e)
    );
  };

  /// revealItemInDir 而不是 openPath：后者不在 opener:default 作用域内，前者无路径限制
  const openDataDir = () => {
    if (!info?.dataDir) return;
    revealItemInDir(info.dataDir).catch((e) => console.error("openDir", e));
  };

  const openLink = (url: string) => {
    openUrl(url).catch((e) => console.error("openUrl", e));
  };

  const checkUpdate = async () => {
    setUpd("checking");
    try {
      const update = await check();
      if (!update) {
        setUpd("latest");
        return;
      }
      setPending(update.version);
      setUpd("installing");
      await update.downloadAndInstall();
      /// 装完必须重启进程才换到新包（Linux 的 AppImage 尤其如此），所以这里直接 relaunch
      await relaunch();
    } catch (e) {
      console.error("updater", e);
      setUpd("error");
    }
  };

  return (
    <div className="dialog-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog dialog-about">
        <div className="dialog-titlebar">
          <span>{t("about.title")}</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body about-body">
          <div className="about-cols">
            <div className="about-brand">
              <img className="about-logo" src="/app-icon.svg" alt="OneCRT" width={64} height={64} />
              <div className="about-name">OneCRT</div>
              <div className="about-ver">{version || "—"}</div>
              <div className="about-env">{envLine || "—"}</div>
              <div className="about-desc">{t("about.desc")}</div>
            </div>
            <div className="about-detail">
              <div className="about-links">
                {ABOUT_LINKS.map((l) => (
                  <button key={l.key} className="about-link" onClick={() => openLink(l.url)}>
                    <span>{t(l.key)}</span>
                    <IconExternalLink {...ink(14)} />
                  </button>
                ))}
              </div>
              <div className="about-dir">
                <span className="about-dir-label">{t("about.dataDir")}</span>
                <button className="btn btn-narrow" disabled={!info?.dataDir} onClick={openDataDir}>
                  {t("about.open")}
                </button>
              </div>
              <div className="about-path" title={info?.dataDir}>
                {info?.dataDir || "—"}
              </div>
              <div className="about-note">{t("about.note")}</div>
              {upd !== "idle" && (
                <div className="about-status">
                  {upd === "checking" && t("about.checking")}
                  {upd === "latest" && t("about.latest")}
                  {upd === "installing" && t("about.installing", { ver: pending })}
                  {upd === "error" && t("about.error")}
                </div>
              )}
            </div>
          </div>
          <div className="about-credits">
            <span>{t("about.builtWith")}</span>
            {ABOUT_CREDITS.map((c, i) => (
              <span key={c.url} className="about-credit-group">
                {i > 0 && <span className="about-dot">·</span>}
                <button className="about-credit" onClick={() => openLink(c.url)}>
                  {c.name}
                </button>
              </span>
            ))}
          </div>
        </div>
        <div className="dialog-footer">
          <button className="btn" onClick={copyInfo} disabled={!version}>
            {copied ? t("about.copied") : t("about.copyInfo")}
          </button>
          <button className="btn" disabled={upd === "checking" || upd === "installing"} onClick={checkUpdate}>
            {t("about.checkUpdate")}
          </button>
          <button className="btn btn-primary" onClick={onClose}>
            {t("common.ok")}
          </button>
        </div>
      </div>
    </div>
  );
}
