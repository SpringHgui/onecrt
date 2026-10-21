import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  IconDeviceDesktopPlus,
  IconFolderPlus,
  IconMoon,
  IconPin,
  IconPinFilled,
  IconPlus,
  IconSettings,
  IconSun,
  IconTrash,
} from "@tabler/icons-react";
import { ink } from "./icons";
import {
  Actions,
  BorderNode,
  DockLocation,
  Layout,
  Model,
  TabGroupNode,
  TabNode,
  TabSetNode,
  type Action,
  type IJsonTabNode,
  type ILayoutApi,
} from "flexlayout-react";
import {
  CENTER,
  PANEL_COMMANDS,
  PANEL_FILES,
  PANEL_SESSIONS,
  clearLayout,
  defaultLayout,
  loadLayout,
  saveLayout,
} from "./layout";
import { CommandNode, Node, SavedCommand, SshSession, isCommandFolder, isFolder, newId } from "./types";
import SessionTree from "./SessionTree";
import QuickConnect, { ConnectParams } from "./QuickConnect";
import PasswordPrompt from "./PasswordPrompt";
import CommandManager from "./CommandManager";
import FilePanel from "./FilePanel";
import CommandProperties, { CommandFields } from "./CommandProperties";
import ContextMenu, { MenuItem } from "./ContextMenu";
import Terminal, { TerminalApi } from "./Terminal";
import { useMenuShortcuts, AboutDialog } from "./MenuBar";
import TitleBar from "./TitleBar";
import Toolbar from "./Toolbar";
import StatusBar from "./StatusBar";
import { buildMenus } from "./menus";
import { useView, useWindowToggles, useZoom, type ViewKey } from "./view";

type TabStatus = "connecting" | "ready" | "error" | "closed";type Theme = "dark" | "light";

function initialTheme(): Theme {
  return localStorage.getItem("shellx.theme") === "light" ? "light" : "dark";
}

interface Tab {
  /// 每个标签页独立，同一会话多开时靠它区分底层连接
  id: string;
  /// 来源会话节点 id；快速连接/克隆出来的标签没有
  sessionId?: string;
  title: string;
  status: TabStatus;
  /// 重连和状态栏都要用
  params?: ConnectParams;
}

type TerminalEvent =
  | { event: "ready" }
  | { event: "output"; data: string }
  | { event: "closed"; reason: string };

/// 会话树与命令树共用的一组纯操作：只依赖 kind/id/name/children 这几个字段，
/// 叶子节点（session / command）没有 children，因此结构上都能套进 AnyNode。
interface AnyNode {
  kind: string;
  id: string;
  name: string;
  children?: AnyNode[];
}

const kids = (n: AnyNode): AnyNode[] => n.children ?? [];
const isBranch = (n: AnyNode) => n.kind === "folder";
const withKids = (n: AnyNode, children: AnyNode[]): AnyNode =>
  isBranch(n) ? { ...n, children } : n;

function mapTree<T extends AnyNode>(nodes: T[], id: string, f: (n: T) => T): T[] {
  return nodes.map((n) => {
    if (n.id === id) return f(n);
    if (isBranch(n)) return { ...n, children: mapTree(kids(n) as T[], id, f) } as T;
    return n;
  });
}

function findNode<T extends AnyNode>(nodes: T[], id: string): T | null {
  for (const n of nodes) {
    if (n.id === id) return n;
    if (isBranch(n)) {
      const hit = findNode(kids(n) as T[], id);
      if (hit) return hit;
    }
  }
  return null;
}

function collectFolderIds(nodes: AnyNode[], out: Set<string>) {
  for (const n of nodes) {
    if (isBranch(n)) {
      if ((n as { expanded?: boolean }).expanded) out.add(n.id);
      collectFolderIds(kids(n), out);
    }
  }
}

function allFolderIds(nodes: AnyNode[], out: Set<string>) {
  for (const n of nodes) {
    if (isBranch(n)) {
      out.add(n.id);
      allFolderIds(kids(n), out);
    }
  }
}

function mapAll<T extends AnyNode>(nodes: T[], f: (n: T) => T): T[] {
  return nodes.map((n) => {
    const m = f(n);
    return isBranch(m) ? ({ ...m, children: mapAll(kids(m) as T[], f) } as T) : m;
  });
}

function removeFromTree<T extends AnyNode>(nodes: T[], id: string): T[] {
  return nodes
    .filter((n) => n.id !== id)
    .map((n) => (isBranch(n) ? ({ ...n, children: removeFromTree(kids(n) as T[], id) } as T) : n));
}

function insertInto<T extends AnyNode>(nodes: T[], folderId: string | null, node: T): T[] {
  if (folderId === null) return [...nodes, node];
  return nodes.map((n) => {
    if (n.id === folderId && isBranch(n)) return { ...n, expanded: true, children: [...kids(n), node] } as T;
    return isBranch(n) ? ({ ...n, children: insertInto(kids(n) as T[], folderId, node) } as T) : n;
  });
}

function parentFolderId<T extends AnyNode>(nodes: T[], targetId: string): string | null {
  function search(list: T[], parent: string | null): string | null | undefined {
    for (const n of list) {
      if (n.id === targetId) return parent;
      if (isBranch(n)) {
        const hit = search(kids(n) as T[], n.id);
        if (hit !== undefined) return hit;
      }
    }
    return undefined;
  }
  const hit = search(nodes, null);
  return hit === undefined ? null : hit;
}

function subtreeHasId(n: AnyNode, id: string): boolean {
  return n.id === id || kids(n).some((c) => subtreeHasId(c, id));
}

function cloneWithNewIds<T extends AnyNode>(n: T): T {
  if (isBranch(n)) return { ...n, id: newId("f"), children: kids(n).map(cloneWithNewIds) } as T;
  return { ...n, id: newId("c") } as T;
}

function filterTree<T extends AnyNode>(nodes: T[], q: string, matches: (n: T) => boolean): T[] {
  const result: T[] = [];
  for (const n of nodes) {
    if (isBranch(n)) {
      const children = filterTree(kids(n) as T[], q, matches);
      if (children.length || n.name.toLowerCase().includes(q)) result.push(withKids(n, children) as T);
    } else if (matches(n)) {
      result.push(n);
    }
  }
  return result;
}

/// "新建文件夹"用的一行占位节点：只存在于显示用的树里，不落盘。
/// 它复用行内改名的那套渲染（id 相同即进入编辑态），提交时才真正建节点。
const NEW_FOLDER_ID = "__new_folder__";
const newSessionFolder = (): Node => ({ kind: "folder", id: NEW_FOLDER_ID, name: "", expanded: false, children: [] });
const newCommandFolder = (): CommandNode => ({
  kind: "folder",
  id: NEW_FOLDER_ID,
  name: "",
  expanded: false,
  children: [],
});

function withNewFolder<T extends AnyNode>(nodes: T[], parentId: string | null, make: () => T): T[] {
  const pid = findNode(nodes, parentId ?? "") ? parentId : null;
  if (pid === null) return [...nodes, make()];
  return nodes.map((n) => {
    if (n.id === pid) return { ...n, children: [...kids(n), make()] } as T;
    if (isBranch(n)) return { ...n, children: withNewFolder(kids(n) as T[], pid, make) } as T;
    return n;
  });
}

type CtxMenu =
  | { kind: "sessions"; node: Node; x: number; y: number }
  | { kind: "commands"; node: CommandNode; x: number; y: number }
  | { kind: "tab"; tabId: string; x: number; y: number };

/// 程序侧发起的布局动作都打这个标记，onModelChange 靠它区分"用户拖的"和"我们自己改的"
const FROM_APP = { fromApp: true };

export default function App() {
  const [tree, setTree] = useState<Node[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [quickConnect, setQuickConnect] = useState<null | { edit?: SshSession; prefill?: Partial<ConnectParams> }>(null);
  const [pwdPrompt, setPwdPrompt] = useState<{ session: SshSession; reconnectTabId?: string } | null>(null);
  const [cmdTree, setCmdTree] = useState<CommandNode[]>([]);
  const [cmdExpanded, setCmdExpanded] = useState<Set<string>>(new Set());
  const [cmdSelectedId, setCmdSelectedId] = useState<string | null>(null);
  const [cmdFilter, setCmdFilter] = useState("");
  const [cmdClipboard, setCmdClipboard] = useState<null | { node: CommandNode; cut: boolean }>(null);
  const [cmdProps, setCmdProps] = useState<null | { node: CommandNode | null; parentId: string | null }>(null);
  const [loaded, setLoaded] = useState(false);
  const [menu, setMenu] = useState<CtxMenu | null>(null);
  const [clipboard, setClipboard] = useState<null | { node: Node; cut: boolean }>(null);
  const [theme, setTheme] = useState<Theme>(initialTheme);
  const { view, toggleView } = useView();
  const { fontSize, zoomIn, zoomOut, zoomReset } = useZoom();
  const { fullScreen, toggleFullScreen, alwaysOnTop, toggleAlwaysOnTop } = useWindowToggles();
  const [about, setAbout] = useState(false);
  const [dims, setDims] = useState<Record<string, { cols: number; rows: number }>>({});
  const filterRef = useRef<HTMLInputElement>(null);
  /// 会话树/命令树里正在行内改名的节点 id（两棵树共用，同时只有一处在编辑）
  const [renamingId, setRenamingId] = useState<string | null>(null);
  /// 新建文件夹：先在树里插一行占位输入框，回车才真正建节点（VSCode 习惯）
  const [newFolder, setNewFolder] = useState<null | { tree: "sessions" | "commands"; parentId: string | null }>(null);
  /// 标签页改名交给 flexlayout 自带的行内编辑
  const layoutRef = useRef<ILayoutApi>(null);

  /// 工作区布局：工具窗口挂四边、终端在中心，可自由拖拽/浮动；布局本身存 localStorage
  const [model, setModel] = useState<Model>(() => Model.fromJson(loadLayout()));
  /// 重置布局要换 Model 实例，Layout 跟着重挂载
  const [layoutEpoch, setLayoutEpoch] = useState(0);
  const tabsRef = useRef<Tab[]>([]);
  tabsRef.current = tabs;

  /// 只在尺寸真的变了才写，免得每次 ResizeObserver 通知都刷一遍状态栏
  const noteDims = (id: string, size: { cols: number; rows: number }) =>
    setDims((d) => (d[id]?.cols === size.cols && d[id]?.rows === size.rows ? d : { ...d, [id]: size }));

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("shellx.theme", theme);
    try {
      // 标题栏由原生窗口主题控制，tauri.conf.json 里写死了 Dark，这里运行时覆盖
      getCurrentWindow()
        .setTheme(theme)
        .catch((e) => console.error("setTheme", e));
    } catch (e) {
      console.error("getCurrentWindow", e);
    }
  }, [theme]);

  const termApis = useRef(new Map<string, TerminalApi>());
  const pendingWrites = useRef(new Map<string, string>());
  const sizeWaiters = useRef(new Map<string, (d: { cols: number; rows: number }) => void>());

  const writeTo = useCallback((id: string, data: string) => {
    const api = termApis.current.get(id);
    if (api) api.write(data);
    else pendingWrites.current.set(id, (pendingWrites.current.get(id) ?? "") + data);
  }, []);

  const registerApi = useCallback((id: string, api: TerminalApi | null) => {
    if (!api) {
      termApis.current.delete(id);
      return;
    }
    termApis.current.set(id, api);
    const buffered = pendingWrites.current.get(id);
    if (buffered) {
      pendingWrites.current.delete(id);
      api.write(buffered);
    }
  }, []);

  /// 终端挂载后才会知道真实列数/行数；等它，否则 PTY 尺寸会停在默认值
  const waitForSize = useCallback(
    (id: string) =>
      new Promise<{ cols: number; rows: number }>((resolve) => {
        const timer = setTimeout(() => {
          sizeWaiters.current.delete(id);
          resolve({ cols: 80, rows: 24 });
        }, 2000);
        sizeWaiters.current.set(id, (d) => {
          clearTimeout(timer);
          sizeWaiters.current.delete(id);
          resolve(d);
        });
      }),
    [],
  );

  useEffect(() => {
    invoke<{ sessions: Node[] }>("load_sessions")
      .then(({ sessions }) => {
        setTree(sessions);
        const set = new Set<string>();
        collectFolderIds(sessions, set);
        setExpanded(set);
      })
      .catch((e) => console.error(e))
      .finally(() => setLoaded(true));
    invoke<{ commands: CommandNode[] }>("load_commands")
      .then(({ commands }) => {
        setCmdTree(commands);
        const set = new Set<string>();
        collectFolderIds(commands, set);
        setCmdExpanded(set);
      })
      .catch(console.error);
  }, []);

  /// 改动会话树的动作一律从这里取当前树：渲染闭包里的 `tree` 可能是旧的，
  /// 连续两次操作（如紧接着的 Paste + Delete）会让后一次写盘丢掉前一次
  const treeRef = useRef<Node[]>([]);
  treeRef.current = tree;
  const cmdRef = useRef<CommandNode[]>([]);
  cmdRef.current = cmdTree;

  const persistTree = useCallback((next: Node[]) => {
    treeRef.current = next;
    setTree(next);
    invoke("save_sessions", { tree: { sessions: next } }).catch(console.error);
  }, []);

  const persistCommands = useCallback((next: CommandNode[]) => {
    cmdRef.current = next;
    setCmdTree(next);
    invoke("save_commands", { tree: { commands: next } }).catch(console.error);
  }, []);

  const updateTab = (id: string, patch: Partial<Tab>) =>
    setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, ...patch } : t)));

  /// 打开（或重开）一个已有 tab 的底层连接。size 已知时不再等终端挂载，
  /// 否则重连会卡到 waitForSize 的 2s 超时、把 PTY 拍回 80x24
  const openTerminal = useCallback(
    async (tabId: string, p: ConnectParams, size?: { cols: number; rows: number }) => {
      const onEvent = new Channel<TerminalEvent>();
      onEvent.onmessage = (ev) => {
        if (ev.event === "ready") updateTab(tabId, { status: "ready" });
        else if (ev.event === "output") writeTo(tabId, ev.data);
        else if (ev.event === "closed") {
          writeTo(tabId, `\r\n\x1b[33m[${ev.reason}]\x1b[0m\r\n`);
          updateTab(tabId, { status: "closed" });
        }
      };

      const { cols, rows } = size ?? (await waitForSize(tabId));
      try {
        await invoke("terminal_open", {
          args: {
            id: tabId,
            host: p.host,
            port: p.port,
            username: p.username,
            password: p.password,
            keyPath: p.keyPath,
            cols,
            rows,
          },
          onEvent,
        });
      } catch (e) {
        updateTab(tabId, { status: "error" });
        writeTo(tabId, `\r\n\x1b[31m${String(e)}\x1b[0m\r\n`);
      }
    },
    [writeTo, waitForSize],
  );

  /// 每次调用都开一个新标签：同一会话允许多开，互不复用
  const connect = useCallback(
    async (p: ConnectParams, sessionId?: string) => {
      const tabId = newId("t");
      setQuickConnect(null);
      setTabs((prev) => [...prev, { id: tabId, sessionId, title: p.name, status: "connecting", params: p }]);
      setActiveTab(tabId);
      // 落点优先当前活动 tabset，这样新连接开在用户正在看的分组里
      const target = model.getActiveTabset()?.getId() ?? model.getFirstTabSet()?.getId() ?? CENTER;
      if (!model.getNodeById(target)) {
        console.error("no tabset to dock new terminal into", target);
      } else {
        model.doAction(
          Actions.addTab(
            { type: "tab", id: tabId, name: p.name, component: "terminal" },
            target,
            DockLocation.CENTER,
            -1,
          ).setUserData(FROM_APP),
        );
      }
      await openTerminal(tabId, p);
    },
    [openTerminal, model],
  );

  const closeTab = useCallback((id: string) => {
    invoke("terminal_close", { id }).catch(() => {});
    termApis.current.delete(id);
    pendingWrites.current.delete(id);
    sizeWaiters.current.delete(id);
    // 布局侧先摘：reconcile 时用户已经关掉了，这里只需处理 App 主动关的情况
    if (model.getNodeById(id)) model.doAction(Actions.deleteTab(id).setUserData(FROM_APP));
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      setActiveTab((cur) => (cur === id ? next[next.length - 1]?.id ?? null : cur));
      return next;
    });
  }, [model]);

  const sendToTab = useCallback((id: string, data: string) => {
    invoke("terminal_send", { id, data }).catch(console.error);
  }, []);

  const visibleTree = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const base = q
      ? filterTree(tree, q, (n) => !isFolder(n) && (n.name.toLowerCase().includes(q) || n.host.toLowerCase().includes(q)))
      : tree;
    return newFolder?.tree === "sessions" ? withNewFolder(base, newFolder.parentId, newSessionFolder) : base;
  }, [tree, filter, newFolder]);

  const visibleCmds = useMemo(() => {
    const q = cmdFilter.trim().toLowerCase();
    const base = q ? filterTree(cmdTree, q, (n) => n.kind === "command" && n.name.toLowerCase().includes(q)) : cmdTree;
    return newFolder?.tree === "commands" ? withNewFolder(base, newFolder.parentId, newCommandFolder) : base;
  }, [cmdTree, cmdFilter, newFolder]);

  const selectedNode = selectedId ? findNode(tree, selectedId) : null;
  const selectedCmd = cmdSelectedId ? findNode(cmdTree, cmdSelectedId) : null;

  const parentFolderIdForNew = (): string | null => {
    if (!selectedNode) return null;
    // 选中会话时挂到其父文件夹；根级会话返回 null，即追加到根
    return isFolder(selectedNode) ? selectedNode.id : parentFolderId(treeRef.current, selectedNode.id);
  };

  const startNewFolder = (which: "sessions" | "commands") => {
    const parentId = which === "sessions" ? parentFolderIdForNew() : cmdParentForNew();
    setNewFolder({ tree: which, parentId });
    setRenamingId(NEW_FOLDER_ID);
    // 占位行要看得见：父文件夹先展开
    if (parentId) (which === "sessions" ? setExpanded : setCmdExpanded)((s) => new Set(s).add(parentId));
  };

  const commitNewFolder = (which: "sessions" | "commands", name: string) => {
    const parentId = newFolder?.parentId ?? null;
    setNewFolder(null);
    setRenamingId(null);
    if (!name) return;
    if (which === "sessions") {
      const node: Node = { kind: "folder", id: newId("f"), name, expanded: true, children: [] };
      persistTree(parentId ? insertInto(treeRef.current, parentId, node) : [...treeRef.current, node]);
      if (parentId) setExpanded((s) => new Set(s).add(parentId));
    } else {
      const node: CommandNode = { kind: "folder", id: newId("cf"), name, expanded: true, children: [] };
      persistCommands(parentId ? insertInto(cmdRef.current, parentId, node) : [...cmdRef.current, node]);
      if (parentId) setCmdExpanded((s) => new Set(s).add(parentId));
    }
  };

  const addFolder = () => startNewFolder("sessions");

  const deleteNode = (node: Node) => {
    if (!window.confirm(`确定删除 "${node.name}" 吗？`)) return;
    persistTree(removeFromTree(treeRef.current, node.id));
    // 删掉文件夹时，它下面已打开的会话标签页也要一起关掉（同一会话可能多开）
    (function closeOpen(n: Node) {
      if (isFolder(n)) n.children.forEach(closeOpen);
      else for (const t of tabs.filter((t) => t.sessionId === n.id)) closeTab(t.id);
    })(node);
    if (selectedId === node.id) setSelectedId(null);
  };

  const onActivate = (node: Node) => {
    if (isFolder(node)) {
      onToggle(node.id);
      return;
    }
    const s = node as SshSession;
    // 后端没有交互式认证，口令/密钥全空的会话先弹框补凭据再连
    if (!s.password && !s.keyPath?.trim()) {
      setPwdPrompt({ session: s });
      return;
    }
    connect({ name: s.name, host: s.host, port: s.port, username: s.username, password: s.password, keyPath: s.keyPath }, s.id);
  };

  /// 口令对话框的 OK / Skip 共用的出口：拼参数 → 按需存会话 → 新标签打开（重连时原地换掉那条连接）
  const startSession = async (
    { session: s, reconnectTabId }: { session: SshSession; reconnectTabId?: string },
    username: string,
    password: string,
    save: boolean,
  ) => {
    setPwdPrompt(null);
    const user = username || s.username;
    if (save && (s.username !== user || s.password !== password)) {
      persistTree(mapTree(treeRef.current, s.id, (n) => ({ ...n, username: user, password } as Node)));
    }
    const p: ConnectParams = { name: s.name, host: s.host, port: s.port, username: user, password, keyPath: s.keyPath };
    if (!reconnectTabId) {
      connect(p, s.id);
      return;
    }
    updateTab(reconnectTabId, { params: p, status: "connecting" });
    // 等旧连接摘掉再开：同一个 tab id 抢在 close 前 open 会被后端判为重复
    await invoke("terminal_close", { id: reconnectTabId }).catch(() => {});
    openTerminal(reconnectTabId, p, dims[reconnectTabId]);
  };

  const onToggle = (id: string) => {
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    persistTree(
      mapTree(treeRef.current, id, (n) => (isFolder(n) ? { ...n, expanded: !expanded.has(id) } : n)),
    );
  };

  const onQuickConnect = (p: ConnectParams, saveSession: boolean) => {
    const tree = treeRef.current;
    if (saveSession && !quickConnect?.edit) {
      const id = newId("s");
      const node: Node = { kind: "session", id, ...p };
      const pid = parentFolderIdForNew();
      persistTree(pid ? mapTree(tree, pid, (n) => (isFolder(n) ? { ...n, expanded: true, children: [...n.children, node] } : n)) : [...tree, node]);
      connect(p, id);
    } else if (quickConnect?.edit) {
      const id = quickConnect.edit.id;
      persistTree(mapTree(tree, id, (n) => ({ ...n, ...p } as Node)));
      connect(p, id);
    } else {
      connect(p);
    }
  };

  const renameNode = (node: Node, raw: string) => {
    const name = raw.trim();
    if (node.id === NEW_FOLDER_ID) {
      commitNewFolder("sessions", name);
      return;
    }
    if (name && name !== node.name) persistTree(mapTree(treeRef.current, node.id, (n) => ({ ...n, name })));
  };

  const pasteNode = (target: Node) => {
    if (!clipboard) return;
    const tree = treeRef.current;
    const dest = isFolder(target) ? target.id : parentFolderId(tree, target.id);
    // 剪切的文件夹不能落进它自己的子树，否则整枝会被上面的移除带走
    if (dest !== null && clipboard.cut && subtreeHasId(clipboard.node, dest)) return;
    const next = insertInto(clipboard.cut ? removeFromTree(tree, clipboard.node.id) : tree, dest, cloneWithNewIds(clipboard.node));
    persistTree(next);
    if (clipboard.cut) setClipboard(null);
  };

  const setAllFoldersExpanded = (open: boolean) => {
    persistTree(mapAll(treeRef.current, (n) => (isFolder(n) ? { ...n, expanded: open } : n)));
    const ids = new Set<string>();
    allFolderIds(treeRef.current, ids);
    setExpanded(open ? ids : new Set());
  };

  const buildMenu = (node: Node): MenuItem[] => {
    const isSession = !isFolder(node);
    const items: MenuItem[] = [];
    if (isSession) {
      items.push(
        { label: "Connect Terminal", accel: "Ctrl+Alt+T", onClick: () => onActivate(node) },
        { label: "Connect in New Window", accel: "Ctrl+Alt+W", disabled: true },
        { label: "Connect in New Tab Group", accel: "Ctrl+Alt+G", disabled: true },
        { label: "Connect File Transfer", accel: "Alt+X", disabled: true },
        { label: "Connect Both", disabled: true },
        {},
      );
    }
    items.push(
      { label: "Cut", accel: "Ctrl+X", onClick: () => setClipboard({ node, cut: true }) },
      { label: "Copy", accel: "Ctrl+C", onClick: () => setClipboard({ node, cut: false }) },
      { label: "Paste", accel: "Ctrl+V", disabled: !clipboard, onClick: () => pasteNode(node) },
      {},
      { label: "Find...", accel: "Ctrl+F", onClick: () => filterRef.current?.focus() },
      { label: "Find Next", accel: "F3", disabled: true },
      {},
      { label: "Delete", accel: "Del", onClick: () => deleteNode(node) },
      { label: "Rename", accel: "F2", onClick: () => setRenamingId(node.id) },
      { label: "Create Desktop Shortcut", accel: "Ctrl+Alt+E", disabled: true },
      {},
      { label: "Expand All Folders", onClick: () => setAllFoldersExpanded(true) },
      { label: "Collapse All Folders", onClick: () => setAllFoldersExpanded(false) },
      {},
      { label: "Arrange Tree", accel: "", hasSubmenu: true, disabled: true },
      { label: "Show Descriptions", checked: true, disabled: true },
      { label: "Show Transfer Sessions", checked: true, disabled: true },
      { label: "Open Sessions in a Tab/Tile", checked: true, disabled: true },
      {},
      isSession
        ? { label: "Properties", accel: "Alt+Enter", onClick: () => setQuickConnect({ edit: node as SshSession }) }
        : { label: "Properties", accel: "Alt+Enter", disabled: true },
    );
    return items;
  };

  const cmdParentForNew = (): string | null => {
    if (!selectedCmd) return null;
    return isCommandFolder(selectedCmd) ? selectedCmd.id : parentFolderId(cmdRef.current, selectedCmd.id);
  };

  const onCmdToggle = (id: string) => {
    setCmdExpanded((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    persistCommands(
      mapTree(cmdRef.current, id, (n) => (isCommandFolder(n) ? { ...n, expanded: !cmdExpanded.has(id) } : n)),
    );
  };

  const cmdAddFolder = () => startNewFolder("commands");

  const cmdSend = (cmd: SavedCommand) => {
    if (activeTab) sendToTab(activeTab, `${cmd.text}\r`);
  };

  const cmdSendTo = (cmd: SavedCommand, targets: Tab[]) => {
    for (const t of targets) sendToTab(t.id, `${cmd.text}\r`);
  };

  const submitCmdProps = (f: CommandFields) => {
    const editing = cmdProps?.node ?? null;
    const parentId = cmdProps?.parentId ?? null;
    if (editing) {
      persistCommands(
        mapTree(cmdRef.current, editing.id, (n) =>
          n.kind === "command" ? { ...n, name: f.name, text: f.text } : { ...n, name: f.name },
        ),
      );
    } else {
      const node: CommandNode = { kind: "command", id: newId("c"), name: f.name, text: f.text };
      persistCommands(insertInto(cmdRef.current, parentId, node));
      if (parentId) setCmdExpanded((s) => new Set(s).add(parentId));
      setCmdSelectedId(node.id);
    }
    setCmdProps(null);
  };

  const deleteCmd = (node: CommandNode) => {
    if (!window.confirm(`确定删除 "${node.name}" 吗？`)) return;
    persistCommands(removeFromTree(cmdRef.current, node.id));
    if (cmdSelectedId === node.id) setCmdSelectedId(null);
  };

  const renameCmd = (node: CommandNode, raw: string) => {
    const name = raw.trim();
    if (node.id === NEW_FOLDER_ID) {
      commitNewFolder("commands", name);
      return;
    }
    if (name && name !== node.name) persistCommands(mapTree(cmdRef.current, node.id, (n) => ({ ...n, name })));
  };

  const pasteCmd = (target: CommandNode) => {
    if (!cmdClipboard) return;
    const cur = cmdRef.current;
    const dest = isCommandFolder(target) ? target.id : parentFolderId(cur, target.id);
    if (dest !== null && cmdClipboard.cut && subtreeHasId(cmdClipboard.node, dest)) return;
    persistCommands(
      insertInto(cmdClipboard.cut ? removeFromTree(cur, cmdClipboard.node.id) : cur, dest, cloneWithNewIds(cmdClipboard.node)),
    );
    if (dest) setCmdExpanded((s) => new Set(s).add(dest));
    if (cmdClipboard.cut) setCmdClipboard(null);
  };

  const setAllCmdFoldersExpanded = (open: boolean) => {
    persistCommands(mapAll(cmdRef.current, (n) => (isCommandFolder(n) ? { ...n, expanded: open } : n)));
    const ids = new Set<string>();
    allFolderIds(cmdRef.current, ids);
    setCmdExpanded(open ? ids : new Set());
  };

  const buildCmdMenu = (node: CommandNode): MenuItem[] => {
    const isCmd = node.kind === "command";
    const live = tabs.filter((t) => t.status === "ready");
    // 当前只有一个 tab group，所以"可见"= 正在显示的那个 pane
    const visible = live.filter((t) => t.id === activeTab);
    return [
      {
        label: "Send Command",
        accel: "Enter",
        disabled: !isCmd || visible.length === 0,
        onClick: () => cmdSend(node as SavedCommand),
      },
      {
        label: "Send Command to",
        disabled: !isCmd || live.length === 0,
        hasSubmenu: true,
        items: [
          {
            label: "All Sessions",
            accel: "Shift+Enter",
            disabled: live.length === 0,
            onClick: () => cmdSendTo(node as SavedCommand, live),
          },
          { label: "Selected Tab Group(s)", disabled: true },
          {
            label: "Visible Sessions",
            disabled: visible.length === 0,
            onClick: () => cmdSendTo(node as SavedCommand, visible),
          },
        ],
      },
      {},
      { label: "Cut", accel: "Ctrl+X", onClick: () => setCmdClipboard({ node, cut: true }) },
      { label: "Copy", accel: "Ctrl+C", onClick: () => setCmdClipboard({ node, cut: false }) },
      { label: "Paste", accel: "Ctrl+V", disabled: !cmdClipboard, onClick: () => pasteCmd(node) },
      {},
      { label: "Delete", accel: "Del", onClick: () => deleteCmd(node) },
      { label: "Rename", accel: "F2", onClick: () => setRenamingId(node.id) },
      {},
      { label: "Expand All Folders", onClick: () => setAllCmdFoldersExpanded(true) },
      { label: "Collapse All Folders", onClick: () => setAllCmdFoldersExpanded(false) },
      {},
      {
        label: "Properties",
        accel: "Alt+Enter",
        onClick: () => setCmdProps({ node, parentId: parentFolderId(cmdRef.current, node.id) }),
      },
    ];
  };

  /// 同一会话多开时按标签顺序加 (1)(2)… 后缀；只开一个时不加，重命名过的名字照样带后缀
  const tabTitles = useMemo(() => {
    const out: Record<string, string> = {};
    const groups = new Map<string, Tab[]>();
    for (const t of tabs) {
      if (!t.sessionId) out[t.id] = t.title;
      else groups.set(t.sessionId, [...(groups.get(t.sessionId) ?? []), t]);
    }
    for (const g of groups.values()) {
      if (g.length < 2) out[g[0].id] = g[0].title;
      else g.forEach((t, i) => (out[t.id] = `${t.title} (${i + 1})`));
    }
    return out;
  }, [tabs]);

  const activeTabObj = tabs.find((t) => t.id === activeTab) ?? null;
  const canSend = activeTabObj?.status === "ready";

  const activeParams = activeTabObj?.params ?? null;
  const statusInfo =
    activeTabObj && activeParams
      ? {
          name: activeTabObj.title,
          status: activeTabObj.status,
          host: activeParams.host,
          port: activeParams.port,
          username: activeParams.username,
          cols: dims[activeTabObj.id]?.cols ?? 0,
          rows: dims[activeTabObj.id]?.rows ?? 0,
        }
      : null;

  const reconnectTab = async (id: string) => {
    const t = tabs.find((x) => x.id === id);
    if (!t?.params) return;
    if (!t.params.password && !t.params.keyPath?.trim()) {
      const node = t.sessionId ? findNode(treeRef.current, t.sessionId) : null;
      if (node && !isFolder(node)) {
        setPwdPrompt({ session: node as SshSession, reconnectTabId: id });
        return;
      }
    }
    updateTab(id, { status: "connecting" });
    await invoke("terminal_close", { id }).catch(() => {});
    openTerminal(id, t.params, dims[id]);
  };

  const disconnectTab = (id: string) => {
    invoke("terminal_close", { id }).catch(console.error);
    updateTab(id, { status: "closed" });
  };

  const reconnectActive = () => activeTab && reconnectTab(activeTab);

  const disconnectActive = () => activeTab && disconnectTab(activeTab);

  const closeTabs = (ids: string[]) => ids.forEach(closeTab);

  const cycleActiveTab = () => {
    if (tabs.length < 2) return;
    const next = tabs[(tabs.findIndex((t) => t.id === activeTab) + 1) % tabs.length];
    setActiveTab(next.id);
    // 终端现在散在多个 tabset 里，只改 App 状态不够，得让布局真的选过去
    model.doAction(Actions.selectTab(next.id).setUserData(FROM_APP));
    termApis.current.get(next.id)?.focus();
  };

  /// 已保存的会话开属性；快速连接型标签没有节点，就带着参数开新建对话框
  const editActiveSession = () => {
    const t = activeTabObj;
    if (!t?.params) return;
    const node = t.sessionId ? findNode(treeRef.current, t.sessionId) : null;
    setQuickConnect(node && !isFolder(node) ? { edit: node as SshSession } : { prefill: t.params });
  };

  /// 标签页原地改名：交给 flexlayout 的行内编辑框，提交后走 RENAME_TAB 回到 updateTab
  const renameTab = (t: Tab) => layoutRef.current?.editTabName(t.id);

  const buildTabMenu = (t: Tab): MenuItem[] => {
    const idx = tabs.findIndex((x) => x.id === t.id);
    const dead = t.status === "closed" || t.status === "error";
    const disconnected = tabs.filter((x) => x.status === "closed" || x.status === "error").map((x) => x.id);
    const node = t.sessionId ? findNode(treeRef.current, t.sessionId) : null;
    const isSavedSession = !!node && !isFolder(node);
    return [
      { label: "Rename", onClick: () => renameTab(t) },
      {
        label: "Reset Name",
        disabled: !t.params || t.title === t.params.name,
        onClick: () => t.params && updateTab(t.id, { title: t.params.name }),
      },
      { label: "Reconnect", disabled: !t.params, onClick: () => reconnectTab(t.id) },
      { label: "Disconnect", disabled: !t.params || dead, onClick: () => disconnectTab(t.id) },
      {},
      { label: "Close", accel: "Ctrl+F4", onClick: () => closeTab(t.id) },
      {
        label: "Close Disconnected Tabs",
        disabled: disconnected.length === 0,
        onClick: () => closeTabs(disconnected),
      },
      {
        label: "Close Other Tabs",
        disabled: tabs.length < 2,
        onClick: () => closeTabs(tabs.filter((x) => x.id !== t.id).map((x) => x.id)),
      },
      {
        label: "Close Tabs to the Right",
        disabled: idx === -1 || idx === tabs.length - 1,
        onClick: () => closeTabs(tabs.slice(idx + 1).map((x) => x.id)),
      },
      { label: "Close Tab Group", disabled: true },
      { label: "Close All Tabs", onClick: () => closeTabs(tabs.map((x) => x.id)) },
      {},
      { label: "Lock Session...", disabled: true },
      {},
      { label: "Clone Session", disabled: !t.params, onClick: () => t.params && connect(t.params, t.sessionId) },
      { label: "Clone in New Window", disabled: true },
      { label: "Send to New Window", disabled: true },
      { label: "Send to New Tab Group", disabled: true },
      {
        label: "Connect SFTP Session",
        disabled: !t.params,
        onClick: () => {
          setActiveTab(t.id);
          if (!view.filePanel) toggleView("filePanel");
        },
      },
      { label: "Open SecureFX", disabled: true },
      {},
      {
        label: "Session Options...",
        disabled: !isSavedSession,
        onClick: () => isSavedSession && setQuickConnect({ edit: node as SshSession }),
      },
      { label: "Save Session...", disabled: true },
      { label: "Send Commands to This Group", disabled: true },
    ];
  };

  /// 三种右键目标（会话树 / 命令树 / 标签页）共用一个 ContextMenu 实例
  const menuItems = (): MenuItem[] => {
    if (!menu) return [];
    if (menu.kind === "sessions") return buildMenu(menu.node);
    if (menu.kind === "commands") return buildCmdMenu(menu.node);
    const t = tabs.find((x) => x.id === menu.tabId);
    return t ? buildTabMenu(t) : [];
  };

  const withActiveApi = (f: (api: TerminalApi) => void) => () => {
    if (activeTab) {
      const api = termApis.current.get(activeTab);
      if (api) f(api);
    }
  };

  /// ---------- 工作区（flexlayout）接线 ----------

  const layoutAction = (a: Action) => model.doAction(a.setUserData(FROM_APP));

  /// 三个工具窗口的固定位置：视图菜单的开关就是往边上加/摘标签
  const panelSlots: { key: ViewKey; id: string; border: string; loc: DockLocation }[] = [
    { key: "sessionManager", id: PANEL_SESSIONS, border: "border_left", loc: DockLocation.LEFT },
    { key: "cmdManager", id: PANEL_COMMANDS, border: "border_left", loc: DockLocation.LEFT },
    { key: "filePanel", id: PANEL_FILES, border: "border_right", loc: DockLocation.RIGHT },
  ];

  const panelJson = (id: string): IJsonTabNode => {
    for (const border of defaultLayout().borders ?? []) {
      for (const child of border.children ?? []) {
        if (child.id === id) return { ...child };
      }
    }
    return { type: "tab", id, name: id, component: id };
  };

  /// 中心区域当前选中的终端；活动 tabset 取不到时退到任意选中的终端标签
  const selectedTerminalId = (m: Model): string | null => {
    const sel = m.getActiveTabset()?.getSelectedNode();
    if (sel && sel.getComponent() === "terminal") return sel.getId();
    const hits: string[] = [];
    m.visitNodes((n) => {
      const t = n as TabNode;
      if (t.getComponent?.() === "terminal" && t.isSelected()) hits.push(t.getId());
    });
    return hits[0] ?? null;
  };

  const onModelChange = (m: Model, action: Action) => {
    const fromApp = (action.userData as { fromApp?: boolean } | undefined)?.fromApp;
    if (!fromApp) {
      // 用户在布局里关掉标签（或整个 tabset）：底层连接要跟着摘掉
      const alive = new Set<string>();
      m.visitNodes((n) => {
        const t = n as TabNode;
        if (t.getComponent?.() === "terminal") alive.add(t.getId());
      });
      for (const t of tabsRef.current) if (!alive.has(t.id)) closeTab(t.id);
      if (action.type === Actions.RENAME_TAB) {
        updateTab(String(action.data.node), { title: String(action.data.text) });
      }
    }
    const next = selectedTerminalId(m);
    setActiveTab((cur) => (cur === next ? cur : next));
    // 拖分隔条时 ADJUST_WEIGHTS 每帧都来，别一帧写一次 localStorage
    if (!action.isAdjusting()) saveLayout(m.toJson());
  };

  useEffect(() => {
    for (const slot of panelSlots) {
      const node = model.getNodeById(slot.id);
      if (view[slot.key] && !node) {
        layoutAction(Actions.addTab(panelJson(slot.id), slot.border, slot.loc, -1, true));
      } else if (!view[slot.key] && node) {
        layoutAction(Actions.deleteTab(slot.id));
      }
    }
  }, [model, view]);

  /// 同一会话多开的 (1)(2) 后缀、菜单里的重命名都要写回布局标签名
  useEffect(() => {
    model.visitNodes((n) => {
      const t = n as TabNode;
      if (t.getComponent?.() !== "terminal") return;
      const want = tabTitles[t.getId()];
      if (want && t.getName() !== want) layoutAction(Actions.renameTab(t.getId(), want));
    });
  }, [model, tabTitles]);

  useEffect(() => {
    // tabEnableRename：右键"Rename"走 flexlayout 的行内编辑框，不再弹窗
    layoutAction(Actions.updateModelAttributes({ tabSetEnableTabStrip: view.sessionTabs, tabEnableRename: true }));
  }, [model, view.sessionTabs]);

  const resetWorkspace = () => {
    // 旧模型整个换掉，里面的终端节点也就没了：先把连接逐个摘干净再换
    for (const t of [...tabsRef.current]) closeTab(t.id);
    clearLayout();
    setModel(Model.fromJson(defaultLayout()));
    setLayoutEpoch((n) => n + 1);
  };

  const renderSessions = () => (
    <div className="dock-fill">
      <div className="sidebar-header">
        <span>Session Manager</span>
      </div>
      <div className="sidebar-toolbar">
        <button title="新建文件夹" onClick={addFolder}><IconFolderPlus {...ink(16)} /></button>
        <button title="新建会话" onClick={() => setQuickConnect({ edit: undefined })}><IconDeviceDesktopPlus {...ink(16)} /></button>
        <button title="会话属性" disabled={!selectedNode || isFolder(selectedNode)} onClick={() => selectedNode && !isFolder(selectedNode) && setQuickConnect({ edit: selectedNode as SshSession })}><IconSettings {...ink(16)} /></button>
        <button title="删除" disabled={!selectedNode} onClick={() => selectedNode && deleteNode(selectedNode)}><IconTrash {...ink(16)} /></button>
        <button
          className="theme-toggle"
          title={theme === "dark" ? "切换到明亮主题" : "切换到深色主题"}
          onClick={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        >
          {theme === "dark" ? <IconSun {...ink(16)} /> : <IconMoon {...ink(16)} />}
        </button>
      </div>
      <input
        ref={filterRef}
        className="filter-input"
        placeholder="Filter by folder/session name"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      <div className="sidebar-tree">
        {loaded && (
          <SessionTree
            nodes={visibleTree}
            expanded={expanded}
            selectedId={selectedId}
            renamingId={renamingId}
            onToggle={onToggle}
            onSelect={setSelectedId}
            onActivate={onActivate}
            onContextMenu={(node, e) => setMenu({ kind: "sessions", node, x: e.clientX, y: e.clientY })}
            onRenameCommit={(node, name) => {
              setRenamingId(null);
              renameNode(node, name);
            }}
            onRenameCancel={() => {
              setRenamingId(null);
              setNewFolder(null);
            }}
          />
        )}
      </div>
    </div>
  );

  const renderCommands = () => (
    <div className="dock-fill">
      <CommandManager
        nodes={visibleCmds}
        expanded={cmdExpanded}
        selectedId={cmdSelectedId}
        renamingId={renamingId}
        hasClipboard={!!cmdClipboard}
        canSend={!!canSend}
        filter={cmdFilter}
        onFilter={setCmdFilter}
        onToggle={onCmdToggle}
        onSelect={setCmdSelectedId}
        onSend={(cmd) => cmdSend(cmd)}
        onNewCommand={() => setCmdProps({ node: null, parentId: cmdParentForNew() })}
        onNewFolder={cmdAddFolder}
        onCut={() => selectedCmd && setCmdClipboard({ node: selectedCmd, cut: true })}
        onCopy={() => selectedCmd && setCmdClipboard({ node: selectedCmd, cut: false })}
        onPaste={() => selectedCmd && pasteCmd(selectedCmd)}
        onDelete={() => selectedCmd && deleteCmd(selectedCmd)}
        onProperties={() =>
          selectedCmd && setCmdProps({ node: selectedCmd, parentId: parentFolderId(cmdRef.current, selectedCmd.id) })
        }
        onContextMenu={(node, e) => setMenu({ kind: "commands", node, x: e.clientX, y: e.clientY })}
        onRenameCommit={(node, name) => {
          setRenamingId(null);
          renameCmd(node, name);
        }}
        onRenameCancel={() => {
          setRenamingId(null);
          setNewFolder(null);
        }}
      />
    </div>
  );

  const renderTerminal = (id: string) => {
    const t = tabs.find((x) => x.id === id);
    if (!t) return null;
    return (
      <div
        className="pane"
        onContextMenu={(e) => {
          // 终端里右键=复制选区（有的话）再粘贴，不弹菜单
          e.preventDefault();
          termApis.current.get(id)?.copyThenPaste();
        }}
      >
        <Terminal
          theme={theme}
          fontSize={fontSize}
          onReady={(size) => {
            noteDims(id, size);
            sizeWaiters.current.get(id)?.(size);
            invoke("terminal_resize", { id, cols: size.cols, rows: size.rows }).catch(() => {});
          }}
          onData={(data) => invoke("terminal_send", { id, data }).catch(console.error)}
          onResize={(cols, rows) => {
            noteDims(id, { cols, rows });
            invoke("terminal_resize", { id, cols, rows }).catch(() => {});
          }}
          apiRef={(api) => registerApi(id, api)}
        />
      </div>
    );
  };

  const factory = (node: TabNode) => {
    const c = node.getComponent();
    if (c === "terminal") return renderTerminal(node.getId());
    if (c === PANEL_SESSIONS) return renderSessions();
    if (c === PANEL_COMMANDS) return renderCommands();
    if (c === PANEL_FILES) {
      return <FilePanel tabId={activeTab} params={activeParams} onClose={() => toggleView("filePanel")} />;
    }
    return null;
  };

  /// 终端标签前面点状态点，替掉原来自绘的 tabbar
  const onRenderTab = (node: TabNode, rv: { leading: React.ReactNode; content: React.ReactNode; buttons: React.ReactNode[] }) => {
    if (node.getComponent() !== "terminal") return;
    const t = tabs.find((x) => x.id === node.getId());
    if (t) rv.leading = <span className={`status-dot ${t.status}`} />;
  };

  /// 钉住 = 面板常驻（border 走 split，主区让位）；取消钉住 = 自动隐藏（overlay，浮在主区上）。
  /// 边缘条的 auto-hide 监听挂在 document 捕获阶段，点工具栏按钮会先把面板收起，
  /// 所以钉住时补一次 selectTab，免得"点了钉住面板反而没了"。
  const toggleBorderPin = (borderId: string) => {
    const border = model.getNodeById(borderId);
    if (!(border instanceof BorderNode)) return;
    const pin = border.getBorderType() !== "split";
    layoutAction(Actions.setBorderType(borderId, pin ? "split" : "overlay"));
    if (pin && !border.getSelectedNode()) {
      const first = border.getTabNodes()[0]?.getId();
      if (first) layoutAction(Actions.selectTab(first));
    }
  };

  /// 边缘按钮条末尾加一枚钉住按钮（VS 工具窗口那个语义，不是 tab 的 sticky）
  const onRenderTabSet = (
    node: TabSetNode | BorderNode,
    rv: {
      leading: React.ReactNode;
      stickyButtons: React.ReactNode[];
      buttons: React.ReactNode[];
      overflowPosition: number | undefined;
    },
  ) => {
    if (!(node instanceof BorderNode) || node.getTabNodes().length === 0) return;
    const pinned = node.getBorderType() === "split";
    rv.buttons.push(
      <button
        key="pin"
        tabIndex={0}
        className="flexlayout__border_toolbar_button pin-toggle"
        title={pinned ? "取消钉住（自动隐藏）" : "钉住面板（常驻）"}
        aria-label={pinned ? "取消钉住" : "钉住面板"}
        aria-pressed={pinned}
        onClick={() => toggleBorderPin(node.getId())}
        onPointerDown={(e) => e.stopPropagation()}
      >
        {pinned ? <IconPinFilled {...ink(14)} /> : <IconPin {...ink(14)} />}
      </button>,
    );
  };

  const onNodeContextMenu = (node: TabNode | TabSetNode | BorderNode | TabGroupNode, e: React.MouseEvent<HTMLElement>) => {
    if (!(node instanceof TabNode) || node.getComponent() !== "terminal") return;
    e.preventDefault();
    e.stopPropagation();
    const t = tabs.find((x) => x.id === node.getId());
    if (t) setMenu({ kind: "tab", tabId: t.id, x: e.clientX, y: e.clientY });
  };

  const menus = buildMenus({
    view,
    theme,
    fontSize,
    fullScreen,
    alwaysOnTop,
    hasTab: !!activeTabObj,
    tabReady: canSend,
    toggleView,
    resetLayout: resetWorkspace,
    toggleTheme: () => setTheme((t) => (t === "dark" ? "light" : "dark")),
    newSession: () => setQuickConnect({ edit: undefined }),
    newFolder: addFolder,
    closeTab: () => activeTab && closeTab(activeTab),
    quit: () =>
      getCurrentWindow()
        .close()
        .catch(console.error),
    reconnect: reconnectActive,
    disconnect: disconnectActive,
    sendInterrupt: () => activeTab && sendToTab(activeTab, "\x03"),
    clearScreen: () => activeTab && sendToTab(activeTab, "\x0c"),
    zoomIn,
    zoomOut,
    zoomReset,
    toggleFullScreen,
    toggleAlwaysOnTop,
    about: () => setAbout(true),
    copyTerminal: withActiveApi((api) => api.copy()),
    pasteTerminal: withActiveApi((api) => api.paste()),
    selectAllTerminal: withActiveApi((api) => api.selectAll()),
  });

  useMenuShortcuts({
    menuBarVisible: view.menuBar,
    onMenuBar: () => toggleView("menuBar"),
    onFullScreen: toggleFullScreen,
    onCloseTab: () => activeTab && closeTab(activeTab),
    onZoomIn: zoomIn,
    onZoomOut: zoomOut,
    onZoomReset: zoomReset,
  });

  return (
    <div className="shell">
      <TitleBar menus={menus} menuVisible={view.menuBar} title={activeTabObj?.title ?? "ShellX"} />
      {view.toolbar && (
        <Toolbar
          newSession={() => setQuickConnect({ edit: undefined })}
          quickConnect={() =>
            setQuickConnect(activeTabObj?.params ? { prefill: activeTabObj.params } : { edit: undefined })
          }
          cycleTab={cycleActiveTab}
          reconnect={reconnectActive}
          copy={withActiveApi((api) => api.copy())}
          paste={withActiveApi((api) => api.paste())}
          sessionOptions={editActiveSession}
          fileTransfer={() => toggleView("filePanel")}
          filePanelOpen={view.filePanel}
          tabCount={tabs.length}
        />
      )}
      <div className="app">
        <div className="workspace">
          <Layout
            ref={layoutRef}
            key={layoutEpoch}
            model={model}
            factory={factory}
            onModelChange={onModelChange}
            onRenderTab={onRenderTab}
            onRenderTabSet={onRenderTabSet}
            onContextMenu={onNodeContextMenu}
            supportsPopout={false}
            onTabSetPlaceHolder={(n) =>
              n.getId() === CENTER ? (
                <div className="empty-state">
                  <p>双击左侧会话或点击 <IconPlus {...ink(13)} /> 新建连接</p>
                </div>
              ) : null
            }
          />
        </div>
      </div>

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems()}
          onClose={() => setMenu(null)}
        />
      )}

      {quickConnect && (
        <QuickConnect
          key={quickConnect.edit?.id ?? `qc-${quickConnect.prefill?.host ?? "new"}`}
          initial={quickConnect.edit ?? quickConnect.prefill}
          showName={quickConnect.edit !== undefined || quickConnect.prefill !== undefined}
          title={quickConnect.edit ? `Session Properties - ${quickConnect.edit.name}` : "Quick Connect"}
          onClose={() => setQuickConnect(null)}
          onConnect={onQuickConnect}
        />
      )}

      {pwdPrompt && (
        <PasswordPrompt
          key={pwdPrompt.session.id}
          host={pwdPrompt.session.host}
          username={pwdPrompt.session.username}
          onClose={() => setPwdPrompt(null)}
          onSkip={(user) => startSession(pwdPrompt, user, "", false)}
          onSubmit={(user, password, save) => startSession(pwdPrompt, user, password, save)}
        />
      )}

      {cmdProps && (
        <CommandProperties
          key={cmdProps.node?.id ?? "new"}
          title={cmdProps.node ? `命令属性 - ${cmdProps.node.name}` : "新建命令"}
          showText={cmdProps.node ? cmdProps.node.kind === "command" : true}
          initial={
            cmdProps.node
              ? { name: cmdProps.node.name, text: cmdProps.node.kind === "command" ? cmdProps.node.text : undefined }
              : undefined
          }
          onClose={() => setCmdProps(null)}
          onSubmit={submitCmdProps}
        />
      )}

      {view.statusBar && <StatusBar info={statusInfo} tabCount={tabs.length} fontSize={fontSize} />}
      {about && <AboutDialog version="0.1.0" onClose={() => setAbout(false)} />}
    </div>
  );
}
