import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

export interface TerminalApi {
  write(data: string): void;
  focus(): void;
  fit(): void;
  copy(): void;
  paste(): void;
  /// 右键用：先把选中的文字复制进剪贴板，再粘贴。选中本身不写剪贴板。
  copyThenPaste(): void;
  selectAll(): void;
}

interface Props {
  theme: "dark" | "light";
  fontSize: number;
  onReady(dims: { cols: number; rows: number }): void;
  onData(data: string): void;
  onResize(cols: number, rows: number): void;
  apiRef(api: TerminalApi | null): void;
}

function readVar(names: string[]): string {
  const s = getComputedStyle(document.documentElement);
  for (const n of names) {
    const v = s.getPropertyValue(n).trim();
    if (v) return v;
  }
  return "";
}

// 与 styles.css 共用一套 token，切主题时两边不会再对不上
function xtermTheme() {
  const red = readVar(["--ansi-red"]);
  const yellow = readVar(["--ansi-yellow"]);
  const green = readVar(["--ansi-green"]);
  return {
    background: readVar(["--bg-content"]),
    foreground: readVar(["--text"]),
    cursor: readVar(["--accent"]),
    selectionBackground: readVar(["--selection", "--bg-selected"]),
    // 报错/断连提示走 31 与 33，默认调色板在深底上对比度不足；亮档没有独立的 9x 档，两处同值
    red,
    brightRed: red,
    yellow,
    brightYellow: yellow,
    green,
    brightGreen: green,
  };
}

// 没有回滚内容时滑块占满整条 1px 槽，紧贴面板分隔线看着像终端多了一圈边框，所以只在真能滚时画
function syncScrollable(host: HTMLElement, term: XTerm) {
  host.classList.toggle("terminal-host--scrollable", term.buffer.active.baseY > 0);
}

export default function Terminal({ theme, fontSize, onReady, onData, onResize, apiRef: setApi }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const apiRef = useRef<TerminalApi | null>(null);
  const readyRef = useRef(false);
  const cbRef = useRef({ onReady, onData, onResize });
  cbRef.current = { onReady, onData, onResize };

  useEffect(() => {
    const host = hostRef.current!;
    const term = new XTerm({
      cursorBlink: true,
      fontFamily: "Cascadia Code, Consolas, 'Courier New', monospace",
      fontSize,
      scrollback: 8000,
      theme: xtermTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;

    // 剪贴板一律走原生命令：navigator.clipboard 在 WebView2 里读要弹权限框，窗口失焦还会失败
    const copySelection = () => {
      const sel = term.getSelection();
      // 空选区什么都不写，右键的"复制+粘贴"才不会把剪贴板清空
      if (!sel) return Promise.resolve();
      return invoke("clipboard_write", { text: sel }).catch((e) => console.error("copy", e));
    };

    const pasteClipboard = () =>
      invoke<string>("clipboard_read")
        .then((t) => t && term.paste(t))
        .catch((e) => {
          console.error("paste", e);
        });

    apiRef.current = {
      write: (d) => term.write(d),
      focus: () => term.focus(),
      fit: () => fit.fit(),
      copy: () => void copySelection(),
      paste: () => void pasteClipboard(),
      copyThenPaste: () => {
        // 必须串起来：写入落到剪贴板之前读到的还是上一次的内容
        copySelection().then(pasteClipboard);
      },
      selectAll: () => term.selectAll(),
    };
    setApi(apiRef.current);

    // xterm 默认把 Ctrl+C/Ctrl+V 当成 ^C/^V 控制字符直接发进 PTY，所以这里先拦住：
    // 粘贴走原生剪贴板（preventDefault 顺手挡掉浏览器自己的 paste，避免贴两遍），
    // 复制按主流终端的做法做成"有选区才是复制"，无选区时 Ctrl+C 仍然发 SIGINT。
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown" || e.altKey) return true;
      const ctrl = e.ctrlKey || e.metaKey;
      // Ctrl+V 与 Shift+Insert 都是粘贴
      if ((ctrl && e.code === "KeyV") || (!ctrl && e.shiftKey && e.code === "Insert")) {
        e.preventDefault();
        void pasteClipboard();
        return false;
      }
      if (!ctrl) return true;
      if (e.shiftKey && e.code === "KeyA") {
        term.selectAll();
        return false;
      }
      if (e.code === "Insert") {
        void copySelection();
        return false;
      }
      if (e.code !== "KeyC") return true;
      if (term.hasSelection()) {
        copySelection();
        return false;
      }
      // Ctrl+Shift+C 是纯复制键，不该退化成中断；Ctrl+C 无选区时放行给 PTY 发 SIGINT
      return !e.shiftKey;
    });

    term.onData((d) => cbRef.current.onData(d));
    term.onResize(({ cols, rows }) => cbRef.current.onResize(cols, rows));
    // term.paste() 的内容也是从 onData 出去的，别再另挂 paste 监听，否则会贴两遍

    const sync = () => syncScrollable(host, term);
    term.onWriteParsed(sync);
    term.onScroll(sync);
    sync();

    const notify = () => cbRef.current.onReady({ cols: term.cols, rows: term.rows });
    if (!readyRef.current) {
      readyRef.current = true;
      notify();
    }
    // 连续改尺寸（拖文件面板分隔条、拖窗口边框）时不能每次交付都 fit：几十个 resize 灌进 PTY，
    // 远端 bash 的逐行重绘会和本地列数对不上，屏幕就花掉。突发内的交付全部合并成最后一次，
    // 静止 GAP 之后才真正 fit（单次改窗口也只多等这么久）。
    const GAP = 120;
    let timer: number | undefined;
    const apply = () => {
      // 面板隐藏时宿主是 0x0，FitAddon 会据此算出 2x1 并 resize 到 PTY，切回来才恢复
      if (!host.clientWidth || !host.clientHeight) return;
      fit.fit();
      sync();
      notify();
    };
    const ro = new ResizeObserver(() => {
      window.clearTimeout(timer);
      timer = window.setTimeout(apply, GAP);
    });
    ro.observe(host);
    term.focus();

    return () => {
      window.clearTimeout(timer);
      ro.disconnect();
      apiRef.current = null;
      setApi(null);
      term.dispose();
    };
  }, []);

  // data-theme 由 App 在渲染阶段写好，这里读到的已是新主题的变量值
  useEffect(() => {
    const term = termRef.current;
    if (term) term.options.theme = xtermTheme();
  }, [theme]);

  // 字号变了宿主尺寸不一定变（滚动条除外），ResizeObserver 靠不住，自己 fit 一次
  useEffect(() => {
    const term = termRef.current;
    const fit = fitRef.current;
    const host = hostRef.current;
    if (!term || !fit) return;
    term.options.fontSize = fontSize;
    if (host?.clientWidth && host?.clientHeight) {
      fit.fit();
      syncScrollable(host, term);
      cbRef.current.onReady({ cols: term.cols, rows: term.rows });
    }
  }, [fontSize]);

  return <div className="terminal-host" ref={hostRef} />;
}
