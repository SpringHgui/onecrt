import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import { invoke, Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  IconChartArea,
  IconDeviceDesktopPlus,
  IconFolder,
  IconFolderPlus,
  IconMoon,
  IconServer,
  IconSettings,
  IconSun,
  IconTerminal2,
  IconTrash,
} from "@tabler/icons-react";
import { ink } from "./icons";
import { startDockStripFit } from "./dockStripFit";
import {
  Actions,
  BorderNode,
  DockLocation,
  Layout,
  Model,
  Rect,
  RowNode,
  TabGroupNode,
  TabNode,
  TabSetNode,
  type Action,
  type ILayoutApi,
} from "flexlayout-react";
import {
  CENTER,
  CENTER_WEIGHT,
  DOCK_TABSET,
  PANEL_COMMANDS,
  PANEL_FILES,
  PANEL_PERF,
  PANEL_SESSIONS,
  PANEL_TAB_KEY,
  borderId,
  BORDER_MAX,
  BORDER_MIN,
  BORDER_SIZE,
  clearLayout,
  defaultLayout,
  loadLayout,
  loadPanelDock,
  panelTabJson,
  saveLayout,
  savePanelDock,
  type DockSide,
  type PanelDockState,
} from "./layout";
import { CommandNode, Node, SavedCommand, SshSession, defaultAuth, enabledMethods, isCommandFolder, isFolder, newId } from "./types";
import SessionTree from "./SessionTree";
import QuickConnect, { ConnectParams } from "./QuickConnect";
import PasswordPrompt from "./PasswordPrompt";
import ConfirmDialog from "./ConfirmDialog";
import ImportDialog from "./ImportDialog";
import McpDialog from "./McpDialog";
import CommandManager from "./CommandManager";
import FilePanel from "./FilePanel";
import PerfPanel from "./PerfPanel";
import PanelPin from "./PanelPin";
import FloatCaptionControls from "./FloatCaption";
import DockCompass, { compassTargetAt, type CompassTarget } from "./DockCompass";
import CommandProperties, { CommandFields } from "./CommandProperties";
import ContextMenu, { MenuItem } from "./ContextMenu";
import Terminal, { TerminalApi } from "./Terminal";
import { useMenuShortcuts, AboutDialog } from "./MenuBar";
import TitleBar from "./TitleBar";
import Toolbar from "./Toolbar";
import StatusBar from "./StatusBar";
import { buildMenus } from "./menus";
import { useView, useWindowToggles, useZoom, type ViewKey } from "./view";
import { useI18n, t, LOCALES, tLocale } from "./i18n";

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

const treeHasSession = (nodes: Node[]): boolean => nodes.some((n) => !isFolder(n) || treeHasSession(n.children));

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

/// 面板标题栏右上角的控件：钉住/自动隐藏都给出图钉（状态看宿主是 tabset 还是边条）
type HeaderCtl = { pinned?: boolean; onTogglePin?: () => void };

/// 程序侧发起的布局动作都打这个标记，onModelChange 靠它区分"用户拖的"和"我们自己改的"
const FROM_APP = { fromApp: true };

/// 工具窗的 tab id：整组标签都是面板的 tabset 就是某条边的钉住宿主。
/// 传输区不在列——它固定在文件面板底部，不再单独占一条边。
const PANEL_TAB_IDS: Set<string> = new Set([PANEL_SESSIONS, PANEL_COMMANDS, PANEL_FILES, PANEL_PERF]);

/// 工具窗各自的页签图标。底部页签条挤窄时文字会整条藏掉，这个图标就是唯一的身份（见 dockStripFit），
/// 所以图形要能一眼对上面板本身：会话=服务器、命令=终端、文件=目录、性能=面积折线。
const PANEL_TAB_ICON: Record<string, ComponentType<{ size: number; stroke: number }>> = {
  [PANEL_SESSIONS]: IconServer,
  [PANEL_COMMANDS]: IconTerminal2,
  [PANEL_FILES]: IconFolder,
  [PANEL_PERF]: IconChartArea,
};

function isPanelDockTabset(ts: TabSetNode): boolean {
  if (ts.getId() === CENTER) return false;
  if (Object.values(DOCK_TABSET).includes(ts.getId())) return true;
  const kids = ts.getChildren();
  return kids.length > 0 && kids.every((c) => PANEL_TAB_IDS.has(c.getId()));
}

/// flexlayout 的 Node 联合类型没把 getWeight 声明齐（RowNode/TabSetNode 上都有），统一这样取
const nodeWeight = (n: unknown): number => Math.max(0, Number((n as { getWeight?: () => number }).getWeight?.()) || 0);

/// 新终端的落点：优先用户正在看的分组，但工具窗宿主除外——双击左侧会话树里的节点时，
/// 活动 tabset 恰恰是会话管理器所在的那条边，直接插进去会让终端和面板并排成同一层标签。
function terminalDropTarget(m: Model): string {
  const ts = m.getActiveTabset();
  if (ts instanceof TabSetNode && !isPanelDockTabset(ts)) return ts.getId();
  return m.getNodeById(CENTER) ? CENTER : (m.getFirstTabSet()?.getId() ?? CENTER);
}

/// flexlayout 自带的 UI 串是英文，浮窗那条抓手的 tooltip 尤其容易被忽略。
/// 没列到的 key 一律原样返回，库会退回它自己的英文默认值；标签名也走这个函数，所以绝不能改写入参。
const FL_I18N: Record<string, string> = {
  "flexlayout.ui.dock.float.to.layout": "fl.dockFloatToLayout",
  "flexlayout.ui.dock.float.tabs (?)": "fl.dockFloatTabs",
};
const flTranslate = (key: string) => (FL_I18N[key] ? t(FL_I18N[key]) : key);

/// 用户拖放劈出来的新 tabset 不带尺寸下限：它只按 weight 和邻居分空间，邻居（钉住 dock 的
/// minWidth 220/280）一挤，新拖进来那栏实测只剩 164px，终端里外就是一条缝，看着像没落下去/被隐藏。
/// 给"这一轮才出现"的 tabset 补一个下限，flexbox 里 min 赢过 flexGrow，让位的会是邻居。
const PANE_MIN_WIDTH = 200;
const PANE_MIN_HEIGHT = 140;

/// seen 是上一轮主布局的 tabset id；没见过的就是刚劈出来的。首轮只登记不补写，
/// 免得存档里那些自带 thickness 的 dock 宿主被这里的通用下限改写。
/// 已经自带 minWidth/minHeight 的节点（dockTabset 造的宿主用 BORDER_MIN）不覆盖，只补缺的那一项。
/// 补写会再触发一次 onModelChange，这里先把 id 记进集合再发动作，重复补写会被跳过。
function floorNewPanes(m: Model, seen: Set<string> | null): Set<string> {
  const now = new Set<string>();
  m.visitNodes((n) => {
    if (n instanceof TabSetNode && n.getLayoutId() === Model.MAIN_LAYOUT_ID) now.add(n.getId());
  });
  if (!seen) return now;
  for (const id of now) {
    if (seen.has(id)) continue;
    seen.add(id);
    const n = m.getNodeById(id);
    if (!(n instanceof TabSetNode)) continue;
    const attrs: Record<string, number> = {};
    if (n.getAttributeOwn("minWidth") === undefined) attrs.minWidth = PANE_MIN_WIDTH;
    if (n.getAttributeOwn("minHeight") === undefined) attrs.minHeight = PANE_MIN_HEIGHT;
    if (Object.keys(attrs).length === 0) continue;
    m.doAction(Actions.updateNodeAttributes(id, attrs).setUserData(FROM_APP));
  }
  return now;
}

/// 0 标签的 tabset 全拆掉。库开着 tabSetEnableDeleteWhenEmpty:false 不会自删，
/// 面板/终端浮出去或搬走之后宿主就成空壳（用户截图里"一堆空白容器"就是这么攒的）。
/// 主布局两类除外：border 上的（空了会自动隐藏，拆了会破坏 border 结构）和 CENTER 本身
/// （没终端时靠它显示"双击会话新建连接"的空态文案）。
/// 浮窗里的空壳照拆：拆掉最后一个 tabset 之后库的 tidy 会把整条 subLayout 从 model 里删掉，
/// 浮窗跟着消失。不拆就剩一具没标签、连顶栏按钮都出不来的浮窗空架（✕ 关掉浮窗面板就这样）。
function sweepEmptyTabsets(m: Model) {
  const dead: string[] = [];
  let deadMain = false;
  m.visitNodes((n) => {
    if (!(n instanceof TabSetNode) || n.getChildren().length > 0) return;
    if (n.getLayoutId() === Model.MAIN_LAYOUT_ID) {
      if (n.getId() === CENTER || n.getParent() instanceof BorderNode) return;
      deadMain = true;
    }
    dead.push(n.getId());
  });
  for (const id of dead) m.doAction(Actions.deleteTabset(id).setUserData(FROM_APP));
  if (deadMain) restoreCenterWeight(m);
}

/// 中心区（装着 CENTER 的最外层容器那一格）和边 dock 抢宽度用的是它自己的 weight，不是 CENTER 的。
/// 库的 row weight 默认 100，边 dock 用 320/460：这一格掉回默认值时中心区只剩 11% 宽，
/// 劈一栏之后每边百来像素，看着像被隐藏了。结构性删改后把它顶回中心区该有的份额。
/// 比的是份额不是原始 weight：拖过一次分隔条，同一行的 weight 会被归一成百分比（24.9 对 75.1），
/// 和出厂那套 320/1000 不同量纲，直接拿数值相比会把 dock 挤成 200px 的边条（或把中心区挤没）。
/// 只动容器不动 CENTER 的权重——中心区内部劈出来的几栏照样等分（拖分隔条走 ADJUST_WEIGHTS，不触发这里）。
function restoreCenterWeight(m: Model) {
  const center = m.getNodeById(CENTER);
  if (!(center instanceof TabSetNode)) return;
  let region: TabSetNode | RowNode = center;
  for (;;) {
    const parent = region.getParent();
    if (!(parent instanceof RowNode) || !parent.getParent()) break; // parent 是根 row：region 就是竞争那一格
    region = parent;
  }
  const row = region.getParent();
  if (!(row instanceof RowNode)) return;
  const own = nodeWeight(region);
  const sumOther = row.getChildren().filter((c) => c !== region).reduce((a, c) => a + nodeWeight(c), 0);
  if (own + sumOther <= 0) return; // 整行都没写过 weight：库按等分算，不会被谁压扁
  const share = own / (own + sumOther);
  const id = region.getId();
  if (!id || share >= 0.55) return;
  const want = Math.round((sumOther * 0.55) / 0.45);
  m.doAction(Actions.updateNodeAttributes(id, { weight: Math.max(want, own) }).setUserData(FROM_APP));
}

/// 旧存档的左右边是 split 且没开自动隐藏：空的时候不收起，屏幕上留一条空白 bar。
/// loadLayout 的归一只管下次加载；这里对活 model 再归一一次，让已经在跑的窗口不等重载就收掉。
function normalizeBordersLive(m: Model) {
  // getBorders 运行时存在但类型标了 internal，这里受控取一次
  const borders = (m.getBorderSet() as unknown as { getBorders(): BorderNode[] }).getBorders();
  for (const b of borders) {
    if (b.isOverlay() && b.isAutoHide()) continue;
    m.doAction(
      Actions.updateNodeAttributes(b.getId(), { borderType: "overlay", enableAutoHide: true }).setUserData(FROM_APP),
    );
  }
}

/// 底部页签条在只有一个页签时是纯冗余（没得切），关掉；加到第二个再放出来。两个方向都当场跟着走。
/// 只动主布局里 tabLocation=bottom 的宿主（停靠面板/翻到底部的页签条），中心区页签在顶部不受影响。
/// 收掉页签不等于没了抓手：面板自己的标题栏就是拖拽把手（见 headerDrag），而往这个宿主里并进第 2 个
/// 面板走的是内容区中心的落点（库的 canDrop 用 contentRect，不看页签条），所以 2→1 无需留手。
function syncDockTabStrips(m: Model, last: Map<string, boolean>) {
  const flips: { id: string; want: boolean }[] = [];
  m.visitNodes((n) => {
    if (!(n instanceof TabSetNode) || n.getLayoutId() !== Model.MAIN_LAYOUT_ID) return;
    if (n.getTabLocation() !== "bottom") return;
    const want = n.getChildren().length > 1;
    if (last.get(n.getId()) === want) return;
    last.set(n.getId(), want);
    flips.push({ id: n.getId(), want });
  });
  for (const f of flips) m.doAction(Actions.updateNodeAttributes(f.id, { enableTabStrip: f.want }).setUserData(FROM_APP));
}

/// 面板页签名是布局 JSON 的一部分，落盘时写的是当时的语言：旧存档、以及从英文界面存出去的布局
/// 里会留着 "Session Manager" 这类英文名。切语言时按 key 换名；只动仍是某语言出厂默认名（或内部 id）
/// 的页签，用户自己改过的名字留着不动。
function syncPanelTabNames(m: Model) {
  m.visitNodes((n) => {
    if (!(n instanceof TabNode)) return;
    const key = PANEL_TAB_KEY[n.getComponent() ?? ""];
    if (!key) return;
    const want = t(key);
    const cur = n.getName();
    if (cur === want) return;
    const factoryNames = [...LOCALES.map((l) => tLocale(l, key)), n.getId()];
    if (factoryNames.includes(cur)) m.doAction(Actions.renameTab(n.getId(), want).setUserData(FROM_APP));
  });
}

/// 模块每次被 HMR 重新求值就 +1。把它挂进收尾 effect 的依赖，热更新后 effect 会重跑一次，
/// 让归一/收单页签条这类收尾逻辑不等重载就在活窗口生效；生产里模块只求值一次，等价于常量。
const HMR_TICK = ((globalThis as { __shellxHmrTick?: number }).__shellxHmrTick =
  ((globalThis as { __shellxHmrTick?: number }).__shellxHmrTick ?? 0) + 1);

export default function App() {
  const { locale, setLocale } = useI18n();
  const [tree, setTree] = useState<Node[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [quickConnect, setQuickConnect] = useState<null | { edit?: SshSession; prefill?: Partial<ConnectParams> }>(null);
  const [pwdPrompt, setPwdPrompt] = useState<{ session: SshSession; reconnectTabId?: string } | null>(null);
  const [confirmBox, setConfirmBox] = useState<{ message: string; action: () => void } | null>(null);
  const [importBox, setImportBox] = useState(false);
  const [mcpBox, setMcpBox] = useState(false);
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
  /// 主题必须在这里（渲染阶段）就落到 <html>，不能放进下面的 effect：
  /// React 先跑子组件的 effect 再跑父组件的，Terminal 读 CSS 变量时会拿到上一档主题。
  document.documentElement.dataset.theme = theme;
  const { view, toggleView, setView } = useView();
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
  /// 上一轮主布局里的 tabset id，用来认出"这一下动作刚劈出来的那栏"并给它补尺寸下限
  const seenTabsetsRef = useRef<Set<string> | null>(null);

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
          writeTo(tabId, `\r\n\x1b[33m[${ev.reason}]\x1b[0m\r\n\x1b[33m${t("term.reconnectHint")}\x1b[0m\r\n`);
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
            methods: enabledMethods(p.auth),
            cols,
            rows,
          },
          onEvent,
        });
      } catch (e) {
        updateTab(tabId, { status: "error" });
        writeTo(tabId, `\r\n\x1b[31m${String(e)}\x1b[0m\r\n\x1b[33m${t("term.reconnectHint")}\x1b[0m\r\n`);
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
      const target = terminalDropTarget(model);
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

  /// 文件 → 导入外部客户端配置：后端弹框选文件并解析，前端补 id 后追加到当前树
  const importSessions = async (cmd: "import_securecrt" | "import_winscp") => {
    let imported: Node[] | null;
    try {
      imported = await invoke<Node[] | null>(cmd);
    } catch (e) {
      setConfirmBox({ message: t("sm.importError", { err: String(e) }), action: () => {} });
      return;
    }
    if (!imported) return; // 用户取消选择

    // 后端把 id 留空，这里统一分配，避免与现有会话/文件夹撞号
    const assign = (nodes: Node[]): Node[] =>
      nodes.map((n) =>
        isFolder(n)
          ? { ...n, id: newId("f"), children: assign(n.children) }
          : { ...n, id: newId("s") },
      );
    const count = (nodes: Node[]): number =>
      nodes.reduce((acc, n) => acc + (isFolder(n) ? count(n.children) : 1), 0);

    const withIds = assign(imported);
    const n = count(withIds);
    if (n === 0) {
      setConfirmBox({ message: t("sm.importNone"), action: () => {} });
      return;
    }
    persistTree([...treeRef.current, ...withIds]);
    setExpanded((s) => {
      const next = new Set(s);
      collectFolderIds(withIds, next);
      return next;
    });
    setConfirmBox({ message: t("sm.imported", { n }), action: () => {} });
  };

  const deleteNode = (node: Node) => {
    setConfirmBox({
      message: t("common.deleteConfirm", { name: node.name }),
      action: () => {
        persistTree(removeFromTree(treeRef.current, node.id));
        // 删掉文件夹时，它下面已打开的会话标签页也要一起关掉（同一会话可能多开）
        (function closeOpen(n: Node) {
          if (isFolder(n)) n.children.forEach(closeOpen);
          else for (const t of tabs.filter((t) => t.sessionId === n.id)) closeTab(t.id);
        })(node);
        if (selectedId === node.id) setSelectedId(null);
      },
    });
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
    connect(
      {
        name: s.name,
        host: s.host,
        port: s.port,
        username: s.username,
        password: s.password,
        keyPath: s.keyPath,
        auth: s.auth ?? defaultAuth(),
        storePassword: !!s.password,
      },
      s.id,
    );
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
    const p: ConnectParams = {
      name: s.name,
      host: s.host,
      port: s.port,
      username: user,
      password,
      keyPath: s.keyPath,
      auth: s.auth ?? defaultAuth(),
      storePassword: save,
    };
    if (!reconnectTabId) {
      connect(p, s.id || undefined);
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

  /// 会话节点落盘字段：口令没勾 Store password 就写空串，"记住密码"的开关真正落到 sessions.json
  const sessionFields = (p: ConnectParams) => ({
    name: p.name,
    host: p.host,
    port: p.port,
    username: p.username,
    password: p.storePassword ? p.password : "",
    keyPath: p.keyPath,
    auth: p.auth,
  });

  const onQuickConnect = (p: ConnectParams, saveSession: boolean) => {
    const tree = treeRef.current;
    let sessionId: string | undefined;
    if (quickConnect?.edit) {
      sessionId = quickConnect.edit.id;
      persistTree(mapTree(tree, sessionId, (n) => ({ ...n, ...sessionFields(p) } as Node)));
    } else if (saveSession) {
      sessionId = newId("s");
      const node: Node = { kind: "session", id: sessionId, ...sessionFields(p) };
      const pid = parentFolderIdForNew();
      persistTree(pid ? mapTree(tree, pid, (n) => (isFolder(n) ? { ...n, expanded: true, children: [...n.children, node] } : n)) : [...tree, node]);
    }
    setQuickConnect(null);
    if (!p.password && !p.keyPath.trim()) {
      // 凭据全空：与双击会话同一条路，先弹口令对话框补齐再连
      setPwdPrompt({ session: { id: sessionId ?? "", ...p, password: "" } });
      return;
    }
    connect(p, sessionId);
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
        { label: t("ctx.connectTerminal"), accel: "Ctrl+Alt+T", onClick: () => onActivate(node) },
        { label: t("ctx.connectNewWindow"), accel: "Ctrl+Alt+W", disabled: true },
        { label: t("ctx.connectNewTabGroup"), accel: "Ctrl+Alt+G", disabled: true },
        { label: t("ctx.connectFileTransfer"), accel: "Alt+X", disabled: true },
        { label: t("ctx.connectBoth"), disabled: true },
        {},
      );
    }
    items.push(
      { label: t("ctx.cut"), accel: "Ctrl+X", onClick: () => setClipboard({ node, cut: true }) },
      { label: t("ctx.copy"), accel: "Ctrl+C", onClick: () => setClipboard({ node, cut: false }) },
      { label: t("ctx.paste"), accel: "Ctrl+V", disabled: !clipboard, onClick: () => pasteNode(node) },
      {},
      { label: t("ctx.find"), accel: "Ctrl+F", onClick: () => filterRef.current?.focus() },
      { label: t("ctx.findNext"), accel: "F3", disabled: true },
      {},
      { label: t("ctx.delete"), accel: "Del", onClick: () => deleteNode(node) },
      { label: t("ctx.rename"), accel: "F2", onClick: () => setRenamingId(node.id) },
      { label: t("ctx.createShortcut"), accel: "Ctrl+Alt+E", disabled: true },
      {},
      { label: t("ctx.expandAll"), onClick: () => setAllFoldersExpanded(true) },
      { label: t("ctx.collapseAll"), onClick: () => setAllFoldersExpanded(false) },
      {},
      { label: t("ctx.arrangeTree"), accel: "", hasSubmenu: true, disabled: true },
      { label: t("ctx.showDescriptions"), checked: true, disabled: true },
      { label: t("ctx.showTransferSessions"), checked: true, disabled: true },
      { label: t("ctx.openSessionsInTab"), checked: true, disabled: true },
      {},
      isSession
        ? { label: t("ctx.properties"), accel: "Alt+Enter", onClick: () => setQuickConnect({ edit: node as SshSession }) }
        : { label: t("ctx.properties"), accel: "Alt+Enter", disabled: true },
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
    setConfirmBox({
      message: t("common.deleteConfirm", { name: node.name }),
      action: () => {
        persistCommands(removeFromTree(cmdRef.current, node.id));
        if (cmdSelectedId === node.id) setCmdSelectedId(null);
      },
    });
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
        label: t("ctx.sendCommand"),
        accel: "Enter",
        disabled: !isCmd || visible.length === 0,
        onClick: () => cmdSend(node as SavedCommand),
      },
      {
        label: t("ctx.sendCommandTo"),
        disabled: !isCmd || live.length === 0,
        hasSubmenu: true,
        items: [
          {
            label: t("ctx.allSessions"),
            accel: "Shift+Enter",
            disabled: live.length === 0,
            onClick: () => cmdSendTo(node as SavedCommand, live),
          },
          { label: t("ctx.selectedTabGroups"), disabled: true },
          {
            label: t("ctx.visibleSessions"),
            disabled: visible.length === 0,
            onClick: () => cmdSendTo(node as SavedCommand, visible),
          },
        ],
      },
      {},
      { label: t("ctx.cut"), accel: "Ctrl+X", onClick: () => setCmdClipboard({ node, cut: true }) },
      { label: t("ctx.copy"), accel: "Ctrl+C", onClick: () => setCmdClipboard({ node, cut: false }) },
      { label: t("ctx.paste"), accel: "Ctrl+V", disabled: !cmdClipboard, onClick: () => pasteCmd(node) },
      {},
      { label: t("ctx.delete"), accel: "Del", onClick: () => deleteCmd(node) },
      { label: t("ctx.rename"), accel: "F2", onClick: () => setRenamingId(node.id) },
      {},
      { label: t("ctx.expandAll"), onClick: () => setAllCmdFoldersExpanded(true) },
      { label: t("ctx.collapseAll"), onClick: () => setAllCmdFoldersExpanded(false) },
      {},
      {
        label: t("ctx.properties"),
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

  const buildTabMenu = (tab: Tab): MenuItem[] => {
    const idx = tabs.findIndex((x) => x.id === tab.id);
    const dead = tab.status === "closed" || tab.status === "error";
    const disconnected = tabs.filter((x) => x.status === "closed" || x.status === "error").map((x) => x.id);
    const node = tab.sessionId ? findNode(treeRef.current, tab.sessionId) : null;
    const isSavedSession = !!node && !isFolder(node);
    return [
      { label: t("ctx.rename"), onClick: () => renameTab(tab) },
      {
        label: t("ctx.resetName"),
        disabled: !tab.params || tab.title === tab.params.name,
        onClick: () => tab.params && updateTab(tab.id, { title: tab.params.name }),
      },
      { label: t("ctx.reconnect"), disabled: !tab.params, onClick: () => reconnectTab(tab.id) },
      { label: t("ctx.disconnect"), disabled: !tab.params || dead, onClick: () => disconnectTab(tab.id) },
      {},
      { label: t("ctx.close"), accel: "Ctrl+F4", onClick: () => closeTab(tab.id) },
      {
        label: t("ctx.closeDisconnected"),
        disabled: disconnected.length === 0,
        onClick: () => closeTabs(disconnected),
      },
      {
        label: t("ctx.closeOther"),
        disabled: tabs.length < 2,
        onClick: () => closeTabs(tabs.filter((x) => x.id !== tab.id).map((x) => x.id)),
      },
      {
        label: t("ctx.closeRight"),
        disabled: idx === -1 || idx === tabs.length - 1,
        onClick: () => closeTabs(tabs.slice(idx + 1).map((x) => x.id)),
      },
      { label: t("ctx.closeTabGroup"), disabled: true },
      { label: t("ctx.closeAll"), onClick: () => closeTabs(tabs.map((x) => x.id)) },
      {},
      { label: t("ctx.lockSession"), disabled: true },
      {},
      { label: t("ctx.cloneSession"), disabled: !tab.params, onClick: () => tab.params && connect(tab.params, tab.sessionId) },
      { label: t("ctx.cloneNewWindow"), disabled: true },
      { label: t("ctx.sendToNewWindow"), disabled: true },
      { label: t("ctx.sendToNewTabGroup"), disabled: true },
      {
        label: t("ctx.connectSftp"),
        disabled: !tab.params,
        onClick: () => {
          setActiveTab(tab.id);
          if (!view.filePanel) toggleView("filePanel");
        },
      },
      { label: t("ctx.openSecureFX"), disabled: true },
      {},
      {
        label: t("ctx.sessionOptions"),
        disabled: !isSavedSession,
        onClick: () => isSavedSession && setQuickConnect({ edit: node as SshSession }),
      },
      { label: t("ctx.saveSession"), disabled: true },
      { label: t("ctx.sendCommandsToGroup"), disabled: true },
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

  /// 面板的默认归属边，以及边名到库枚举的映射（DockLocation.getByName 是 internal，不能用）
  const DEFAULT_SIDE: Record<string, DockSide> = {
    [PANEL_SESSIONS]: "left",
    [PANEL_COMMANDS]: "left",
    [PANEL_FILES]: "right",
    [PANEL_PERF]: "right",
  };
  const DOCK_LOC: Record<DockSide, DockLocation> = {
    left: DockLocation.LEFT,
    right: DockLocation.RIGHT,
    top: DockLocation.TOP,
    bottom: DockLocation.BOTTOM,
  };

  /// 用户把面板拖到哪条边、是否自动隐藏，都记下来；视图菜单重开时还原到原处而不是默认边
  const panelDock = useRef<Record<string, PanelDockState>>(loadPanelDock());

  /// 已经补过名字的浮窗，onModelChange 里按 POPOUT_TAB 逐个登记
  const floatedRef = useRef<Set<string>>(new Set());

  /// 主布局里被认领的"钉住宿主"tabset：id -> 边。defaultLayout 建的、和用户拖出来新劈的都在内。
  /// 认领表里的 tabset 才算钉住面板的合法住所；不在表里的主区 tabset（比如浮窗并回主区劈出的
  /// 半个）按游荡处理，由收尾逻辑挪走。
  const dockedRef = useRef<Map<string, DockSide>>(new Map());

  /// 浮窗并回主区时，库会把面板塞进它自己挑的宿主（常常是就近的边 tabset，跟"记住的边"无关），
  /// 而且 DOCK_FLOAT_TO_LAYOUT 触发 onModelChange 那一刻面板可能还没落到主布局里。
  /// 先把这些面板记进待归位集合，等它在主布局露面后（effect）再按记忆的边搬回去。
  const pendingRestoreRef = useRef<Set<string>>(new Set());

  const rememberDock = (id: string, side: DockSide, hidden: boolean, frac?: number) => {
    const cur = panelDock.current[id];
    const f = frac ?? cur?.frac;
    if (cur && cur.side === side && cur.hidden === hidden && cur.frac === f) return;
    panelDock.current = { ...panelDock.current, [id]: { side, hidden, frac: f } };
    savePanelDock(panelDock.current);
  };
  const dockState = (id: string): PanelDockState => panelDock.current[id] ?? { side: DEFAULT_SIDE[id], hidden: false };

  /// 面板现在这个宿主在它那一行里占的份额。拔钉之前量一次，整条边清空、宿主被扫掉之后按它复原厚度。
  /// 只在这条行里比 weight 才有意义（它们是同一量纲的相对值），所以取份额不取绝对 weight。
  const hostFrac = (m: Model, id: string): number | undefined => {
    const node = m.getNodeById(id);
    const host = node instanceof TabNode ? node.getParent() : undefined;
    if (!(host instanceof TabSetNode) || host.getId() === CENTER) return undefined;
    const row = host.getParent();
    if (!row) return undefined;
    const sibs = row.getChildren();
    const sum = sibs.reduce((a, c) => a + nodeWeight(c), 0);
    const own = nodeWeight(host);
    if (sum <= 0 || own <= 0) return undefined;
    return Math.min(0.8, own / sum);
  };

  /// 四个工具窗：视图菜单的开关就是往它当前那条边加/摘标签
  const panelSlots: { key: ViewKey; id: string }[] = [
    { key: "sessionManager", id: PANEL_SESSIONS },
    { key: "cmdManager", id: PANEL_COMMANDS },
    { key: "filePanel", id: PANEL_FILES },
    { key: "perfPanel", id: PANEL_PERF },
  ];

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
    // 浮窗的名字与标签条位置：库建出来的浮窗没名字（标题栏只有一条灰线），标签条还在顶上跟面板
    // 自己的标题行打架。照 VS 的浮窗：名字进窗口顶栏、标签条翻到底部（内容区就不再需要标题行了）。
    // 拖出去那条路和库自己的浮动按钮都走 POPOUT_TAB。model 是同一个引用，挂 useEffect 不会重跑，
    // 只能在这里跟着动作走。别改 enableTabStrip：那属性长在节点上，抓手把浮窗拖回主布局时会跟着回来。
    if (action.type === Actions.POPOUT_TAB) {
      const node = m.getNodeById(String(action.data.node)) as TabNode | undefined;
      const lid = node?.getLayoutId();
      if (node && lid && lid !== Model.MAIN_LAYOUT_ID && !floatedRef.current.has(lid)) {
        floatedRef.current.add(lid);
        const acts = [Actions.updateSubLayoutAttributes(lid, { name: node.getName() })];
        const host = node.getParent();
        if (host instanceof TabSetNode) acts.push(Actions.updateNodeAttributes(host.getId(), { tabLocation: "bottom" }));
        m.doAction(Actions.group(acts).setUserData(FROM_APP));
      }
    }
    // 浮窗并回主布局只有 DOCK_FLOAT_TO_LAYOUT 这一条动作，顶栏按钮和库那个抓手都走它，
    // 而库只会把浮窗的 tabset 劈进主区中心——面板类工具窗留在中心就是多出来的第二个 tabset。
    // 所以在这里统一收尾：按记忆把面板放回钉住宿主或边条，劈出来又被搬空的 tabset 交给清扫；
    // 浮窗里翻到底部的标签条也要翻回顶上（住面板的底标签条除外，那是 VS 的页签位置）。终端不在此列，回中心是对的。
    if (action.type === Actions.DOCK_FLOAT_TO_LAYOUT) {
      // 库把浮窗劈进主区，落点跟"记住的边"无关（常就近并进某个边 tabset），而且这一刻面板可能还没落到主布局。
      // 记进待归位集合，交给 effect 在主布局稳定后按记忆的边搬回；这里先不碰 store。
      for (const slot of panelSlots) pendingRestoreRef.current.add(slot.id);
      const bottoms: string[] = [];
      m.visitNodes((n) => {
        if (
          n instanceof TabSetNode &&
          n.getLayoutId() === Model.MAIN_LAYOUT_ID &&
          n.getTabLocation() === "bottom" &&
          !n.getChildren().some((c) => panelSlots.some((s) => s.id === c.getId()))
        ) {
          bottoms.push(n.getId());
        }
      });
      for (const id of bottoms) m.doAction(Actions.updateNodeAttributes(id, { tabLocation: "top" }).setUserData(FROM_APP));
    }
    // 面板落在哪 = 它的停靠状态：边条里 = 自动隐藏；主区边缘 tabset 里 = 钉住。
    // 只有"用户亲手把这个面板拖过去"（MOVE_NODE 的主体就是它）才更新记忆；浮窗并回、清扫、切终端等
    // 附带动作不改 store，否则库把浮窗塞进的错误宿主会把记忆的边冲掉。
    const movedId = action.type === Actions.MOVE_NODE ? String((action.data as { fromNode?: unknown })?.fromNode ?? "") : "";
    for (const slot of panelSlots) {
      if (pendingRestoreRef.current.has(slot.id)) continue;
      if (slot.id !== movedId) continue;
      const node = m.getNodeById(slot.id);
      if (!(node instanceof TabNode) || node.getLayoutId() !== Model.MAIN_LAYOUT_ID) continue;
      const parent = node.getParent();
      if (parent instanceof BorderNode) {
        rememberDock(slot.id, parent.getLocation().getName() as DockSide, true);
      } else if (parent instanceof TabSetNode && parent.getId() !== CENTER) {
        let known = dockedRef.current.get(parent.getId());
        // 上一轮还没见过这个 tabset id = 这一下拖放刚劈出来的新栏，得给它配该边的厚度
        const brandNew = !seenTabsetsRef.current?.has(parent.getId());
        if (!known) {
          // 新劈的 tabset DOM 还没重画，量几何会读到旧位置：优先信这一下动作里带的边
          known = dropEdge(action) ?? sideOfTabset(parent.getId());
          adoptDockTabset(parent, known, brandNew);
        }
        rememberDock(slot.id, known, false, hostFrac(m, slot.id));
      }
    }
    // 浮窗并回：这一刻面板已经落进库挑的宿主，按记忆的边搬回去，再扫掉劈空的壳。
    // 不能挂 useEffect([model])：Model 是同一个引用，改了也不会重跑。
    if (pendingRestoreRef.current.size > 0) {
      let moved = false;
      for (const id of [...pendingRestoreRef.current]) {
        const node = m.getNodeById(id);
        if (!(node instanceof TabNode)) {
          pendingRestoreRef.current.delete(id); // 面板已不存在（被关掉），没什么可归位的
          continue;
        }
        if (node.getLayoutId() !== Model.MAIN_LAYOUT_ID) continue; // 还留在浮窗里，等下一次动作
        pendingRestoreRef.current.delete(id);
        placePanel(m, id);
        moved = true;
      }
      if (moved) sweepEmptyTabsets(m);
    }
    // 浮出去、搬去边、关掉最后一个标签都会把宿主 tabset 搬空，动作落地后立刻扫掉空壳
    if (
      action.type === Actions.POPOUT_TAB ||
      action.type === Actions.MOVE_NODE ||
      action.type === Actions.DELETE_TAB ||
      action.type === Actions.ADD_TAB ||
      action.type === Actions.DOCK_FLOAT_TO_LAYOUT
    ) {
      sweepEmptyTabsets(m);
      // 宿主只剩 1 个面板时当场收掉页签条，并进来第 2 个时再亮出来
      syncDockTabStrips(m, singleStripRef.current);
    }
    // 刚劈出来的那栏没有尺寸下限，会被邻居的 minWidth 挤成一条缝（实测 164px），看着像被隐藏了
    seenTabsetsRef.current = floorNewPanes(m, seenTabsetsRef.current);
  };

  /// 记录每个底部宿主上一次设置的 enableTabStrip，避免收尾 effect 每轮重复下动作
  const singleStripRef = useRef<Map<string, boolean>>(new Map());

  useEffect(() => {
    for (const slot of panelSlots) {
      const node = model.getNodeById(slot.id);
      if (view[slot.key] && !node) {
        const st = dockState(slot.id);
        if (st.hidden) {
          layoutAction(Actions.addTab(panelTabJson(slot.id), borderId(st.side), DOCK_LOC[st.side], -1, false));
          continue;
        }
        const ts = panelTabsetFor(st.side);
        if (ts) layoutAction(Actions.addTab(panelTabJson(slot.id), ts.getId(), DockLocation.CENTER, -1, true));
        else {
          layoutAction(Actions.addTab(panelTabJson(slot.id), CENTER, DOCK_LOC[st.side], -1, true));
          const p = model.getNodeById(slot.id)?.getParent();
          if (p instanceof TabSetNode) adoptDockTabset(p, st.side, true);
        }
      } else if (!view[slot.key] && node) {
        layoutAction(Actions.deleteTab(slot.id));
      }
    }
    // 新开的面板可能落在一株刚劈出来的宿主上（tabLocation 是 adopt 之后才翻到底部的），
    // 那时 ADD_TAB 那一下的同步还看不见它，这里补一次
    syncDockTabStrips(model, singleStripRef.current);
  }, [model, view]);

  /// 底部页签条挤到放不下文字时退成只剩图标。判据要实测宽度，CSS 比不了长度，所以单独一趟扫描。
  useEffect(() => startDockStripFit(), []);

  /// 旧存档是按老规矩存的：面板住在边条里（钉住/隐藏按整条边的 borderType 分），更旧的还有空 tabset、
  /// 停在中心区的面板（用户那屏"一堆空白容器"）。加载时先认领盘上的钉住宿主，再按 store 逐个归位：
  /// 钉住 -> 边 tabset，自动隐藏 -> 边条；收尾动作会触发 saveLayout，落盘即新规矩的干净布局。
  useEffect(() => {
    model.visitNodes((n) => {
      if (
        n instanceof TabSetNode &&
        n.getLayoutId() === Model.MAIN_LAYOUT_ID &&
        n.getId() !== CENTER &&
        !dockedRef.current.has(n.getId()) &&
        n.getChildren().length > 0 &&
        n.getChildren().every((c) => panelSlots.some((s) => s.id === c.getId()))
      ) {
        adoptDockTabset(n, sideOfTabset(n.getId()));
      }
    });
    for (const slot of panelSlots) {
      const node = model.getNodeById(slot.id);
      if (!(node instanceof TabNode) || node.getLayoutId() !== Model.MAIN_LAYOUT_ID) continue;
      const host = node.getParent();
      const st = dockState(slot.id);
      if (st.hidden && host instanceof BorderNode) continue;
      if (!st.hidden && host instanceof TabSetNode && dockedRef.current.has(host.getId())) continue;
      placePanel(model, slot.id);
    }
    sweepEmptyTabsets(model);
    normalizeBordersLive(model);
    syncDockTabStrips(model, singleStripRef.current);
  }, [model, HMR_TICK]);

  /// ---------- 拖出窗口 → 就地变成浮动面板 ----------
  /// 库的 dragend 只清拖拽状态，没有"甩出窗口就浮动"这一路，得自己补。
  /// 判据用"是否离开过文档"而不是 dragend 的坐标：Chrome 在窗口外松手时坐标可能被夹到视口边上，
  /// 光看坐标会漏；relatedTarget 为 null 的 dragleave/dragenter 恰好就是跨越窗口边界的那一对。
  const dragNodeRef = useRef<string | null>(null);
  const dragLeftWindowRef = useRef(false);
  const compassRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    /// 拖拽源有两种：面板标题栏把手（data-node-id）和库自己的标签按钮（flexlayout-tabbutton-<id>）
    const nodeIdOf = (t: EventTarget | null): string | null => {
      const el = (t as Element | null)?.closest?.("[data-node-id],[id^='flexlayout-tabbutton-']");
      const own = el?.getAttribute("data-node-id");
      if (own) return own;
      if (!el) return null;
      // 库的 domId() 把空白换成了下划线，这里换回来
      return el.id.slice("flexlayout-tabbutton-".length).replace(/_/g, " ");
    };
    /// 罗盘直接改 DOM：dragover 每秒几十次，进 React state 会连带重渲染整棵 App（终端在里面）
    const paintCompass = (t: CompassTarget | null) => {
      const el = compassRef.current;
      if (!el) return;
      if (!t) {
        el.hidden = true;
        return;
      }
      el.hidden = false;
      el.style.left = `${t.x}px`;
      el.style.top = `${t.y}px`;
      el.style.width = `${t.w}px`;
      el.style.height = `${t.h}px`;
      el.dataset.zone = t.zone;
      // 标签区比罗盘还小时整体缩一圈，别让箭头戳到区外
      el.style.setProperty("--k", String(Math.min(1, Math.min(t.w, t.h) / 200)));
    };
    const clearDrag = () => {
      dragNodeRef.current = null;
      dragLeftWindowRef.current = false;
      paintCompass(null);
    };
    const onDragStart = (e: DragEvent) => {
      clearDrag();
      dragNodeRef.current = nodeIdOf(e.target);
    };
    const onDragOver = (e: DragEvent) => {
      if (dragNodeRef.current) paintCompass(compassTargetAt(e.clientX, e.clientY));
    };
    const onBoundary = (entered: boolean) => (e: DragEvent) => {
      if (e.relatedTarget === null) dragLeftWindowRef.current = !entered;
    };
    const onDragEnter = onBoundary(true);
    const onDragLeave = onBoundary(false);
    const onDragEnd = (e: DragEvent) => {
      const id = dragNodeRef.current;
      const leftWindow = dragLeftWindowRef.current;
      clearDrag();
      if (!id || !leftWindow) return;
      const root = layoutRef.current?.getRootDiv();
      if (!root || !model.getNodeById(id)) return;
      const r = root.getBoundingClientRect();
      const w = Math.min(460, Math.round(r.width * 0.4));
      const h = Math.min(340, Math.round(r.height * 0.5));
      const clamp = (v: number, hi: number) => Math.min(Math.max(v, 8), Math.max(8, hi));
      // 浮窗矩形是相对布局根节点的局部坐标，松手点当左上角抓握处，夹进窗口内
      const rect = new Rect(
        clamp(e.clientX - r.left - 24, r.width - w - 8),
        clamp(e.clientY - r.top - 16, r.height - h - 8),
        w,
        h,
      );
      // popoutTab 会把浮窗建在标签原位置，紧接着搬到松手处；新 layoutId 从搬家后的节点上读
      layoutAction(Actions.popoutTab(id, "float"));
      const floatId = model.getNodeById(id)?.getLayoutId();
      if (floatId) layoutAction(Actions.moveFloat(floatId, rect));
    };
    /// 收尾不能只等 dragend：落点成功时库改模型，页签按钮跟着被搬进别的 tabset 而卸掉，
    /// 紧跟其后的 dragend 打在脱离文档的节点上，冒不到 window，罗盘就留在屏上。
    const onDrop = () => clearDrag();
    window.addEventListener("dragstart", onDragStart, true);
    window.addEventListener("dragover", onDragOver, true);
    window.addEventListener("dragenter", onDragEnter, true);
    window.addEventListener("dragleave", onDragLeave, true);
    window.addEventListener("dragend", onDragEnd, true);
    window.addEventListener("drop", onDrop, true);
    return () => {
      window.removeEventListener("dragstart", onDragStart, true);
      window.removeEventListener("dragover", onDragOver, true);
      window.removeEventListener("dragenter", onDragEnter, true);
      window.removeEventListener("dragleave", onDragLeave, true);
      window.removeEventListener("dragend", onDragEnd, true);
      window.removeEventListener("drop", onDrop, true);
    };
  }, [model]);

  /// 同一会话多开的 (1)(2) 后缀、菜单里的重命名都要写回布局标签名
  useEffect(() => {
    model.visitNodes((n) => {
      const t = n as TabNode;
      if (t.getComponent?.() !== "terminal") return;
      const want = tabTitles[t.getId()];
      if (want && t.getName() !== want) layoutAction(Actions.renameTab(t.getId(), want));
    });
  }, [model, tabTitles]);

  /// 面板页签名跟着语言换（首轮同时负责把旧存档里的英文名改过来）
  useEffect(() => {
    syncPanelTabNames(model);
  }, [model, locale]);

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

  const renderSessions = (pin?: HeaderCtl, drag?: React.HTMLAttributes<HTMLDivElement>) => (
    <div className="dock-fill">
      <div className="sidebar-header" {...drag}>
        <span>{t("sm.title")}</span>
        {pin?.onTogglePin && <PanelPin pinned={!!pin.pinned} onToggle={pin.onTogglePin} />}
      </div>
      <div className="sidebar-toolbar">
        <button title={t("sm.newFolder")} onClick={addFolder}><IconFolderPlus {...ink(16)} /></button>
        <button title={t("sm.newSession")} onClick={() => setQuickConnect({ edit: undefined })}><IconDeviceDesktopPlus {...ink(16)} /></button>
        <button title={t("sm.sessionProps")} disabled={!selectedNode || isFolder(selectedNode)} onClick={() => selectedNode && !isFolder(selectedNode) && setQuickConnect({ edit: selectedNode as SshSession })}><IconSettings {...ink(16)} /></button>
        <button title={t("sm.delete")} disabled={!selectedNode} onClick={() => selectedNode && deleteNode(selectedNode)}><IconTrash {...ink(16)} /></button>
        <button
          className="theme-toggle"
          title={theme === "dark" ? t("sm.toLight") : t("sm.toDark")}
          onClick={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        >
          {theme === "dark" ? <IconSun {...ink(16)} /> : <IconMoon {...ink(16)} />}
        </button>
      </div>
      <input
        ref={filterRef}
        className="filter-input"
        placeholder={t("sm.filter")}
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
            onDelete={() => selectedNode && deleteNode(selectedNode)}
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

  const renderCommands = (pin?: HeaderCtl, drag?: React.HTMLAttributes<HTMLDivElement>) => (
    <div className="dock-fill">
      <CommandManager
        drag={drag}
        pinned={pin?.pinned ?? false}
        onTogglePin={pin?.onTogglePin ?? (() => {})}
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
          onData={(data) => {
            // 断开后回车 = 重连，与工具栏 Reconnect 走同一条路
            const st = tabs.find((x) => x.id === id)?.status;
            if (data === "\r" && (st === "closed" || st === "error")) {
              reconnectTab(id);
              return;
            }
            invoke("terminal_send", { id, data }).catch(console.error);
          }}
          onResize={(cols, rows) => {
            noteDims(id, { cols, rows });
            invoke("terminal_resize", { id, cols, rows }).catch(() => {});
          }}
          apiRef={(api) => registerApi(id, api)}
        />
      </div>
    );
  };

  /// 面板标题栏就是拖拽把手：在 HTML dragstart 里把节点交给 flexlayout 的拖放管理器，
  /// 之后的落点指示（中心四分 / 四边）走库自己的逻辑，甩出窗口那一路见上面的 dragend 监听。
  /// data-node-id 让那个监听能认出这是哪个节点——它只能看到原生事件的 target。
  const headerDrag = (node: TabNode): React.HTMLAttributes<HTMLDivElement> & { "data-node-id": string } => ({
    draggable: true,
    "data-node-id": node.getId(),
    onDragStart: (e) => layoutRef.current?.moveTabWithDragAndDrop(e.nativeEvent, node),
  });

  /// 传输一开始就把文件面板放出来（传输区就在它底部）。由文件面板回调触发（不是订阅 store）：
  /// 进度 tick 一秒来十几次，让 App 跟着订阅会带着整个工作区重渲染。
  const showTransfers = () =>
    setView((v) => (v.filePanel && v.transfers ? v : { ...v, filePanel: true, transfers: true }));

  const panelContent = (node: TabNode) => {
    const c = node.getComponent();
    const pin = headerCtl(node);
    const drag = headerDrag(node);
    if (c === "terminal") return renderTerminal(node.getId());
    if (c === PANEL_SESSIONS) return renderSessions(pin, drag);
    if (c === PANEL_COMMANDS) return renderCommands(pin, drag);
    if (c === PANEL_FILES) {
      return (
        <FilePanel
          tabId={activeTab}
          params={activeParams}
          onClose={() => toggleView("filePanel")}
          onTransferStart={showTransfers}
          transfersOpen={view.transfers}
          pinned={pin?.pinned}
          onTogglePin={pin?.onTogglePin}
          drag={drag}
        />
      );
    }
    if (c === PANEL_PERF) {
      return (
        <PerfPanel
          tabId={activeTab}
          params={activeParams}
          onClose={() => toggleView("perfPanel")}
          pinned={pin?.pinned}
          onTogglePin={pin?.onTogglePin}
          drag={drag}
        />
      );
    }
    return null;
  };

  /// 浮窗回主窗口。库只给了两条路：标题栏那个 18px 抓手拖回主区（落点只能是主区的 tabset/row，
  /// 既不认 CENTER 也不认 border），或者 nothing。两条路最终都是 DOCK_FLOAT_TO_LAYOUT，
  /// "回哪条边、拆空 tabset"的收尾统一在 onModelChange 里做，这里只负责把整窗并回主区
  /// （顺带删掉 subLayout，否则留一个空浮窗壳）。用 moveNode 而不是 delete+addTab，终端标签的连接才不会断。
  const dockBack = (id: string) => {
    const node = model.getNodeById(id);
    if (!(node instanceof TabNode)) return;
    const floatId = node.getLayoutId();
    if (floatId === Model.MAIN_LAYOUT_ID) return;
    const target = model.getNodeById(CENTER) ?? model.getRootRow();
    if (!target) return;
    layoutAction(Actions.dockFloatToLayout(floatId, target.getId(), DockLocation.TOP, -1));
    // 面板内容走 TabContentRenderer(React.memo)，跨布局搬完不会重画：不 redraw 的话浮窗那条顶栏
    // 会跟着面板一起回到边上，图钉也出不来
    layoutRef.current?.redraw();
  };

  /// 浮窗顶栏的 ✕：整窗关掉。面板走视图开关（直接 deleteTab 会被 view 同步又加回来），终端直接删标签
  const closeFloat = (ids: string[]) => {
    for (const id of ids) {
      const slot = panelSlots.find((s) => s.id === id);
      if (slot) toggleView(slot.key);
      else layoutAction(Actions.deleteTab(id));
    }
  };

  /// 终端标签前面点状态点，替掉原来自绘的 tabbar；工具窗标签放各自的图标
  const onRenderTab = (node: TabNode, rv: { leading: React.ReactNode; content: React.ReactNode; buttons: React.ReactNode[] }) => {
    const c = node.getComponent();
    if (c === "terminal") {
      const t = tabs.find((x) => x.id === node.getId());
      if (t) rv.leading = <span className={`status-dot ${t.status}`} />;
      return;
    }
    const Icon = PANEL_TAB_ICON[c ?? ""];
    if (Icon) rv.leading = <Icon {...ink(14)} />;
  };

  /// 某条边的钉住宿主 tabset：认领表里边 -> id；节点已被当空壳扫掉时顺手除名
  const panelTabsetFor = (side: DockSide): TabSetNode | undefined => {
    for (const [tid, s] of dockedRef.current) {
      if (s !== side) continue;
      const n = model.getNodeById(tid);
      if (n instanceof TabSetNode) return n;
      dockedRef.current.delete(tid);
    }
    return undefined;
  };

  /// 认领一个钉住宿主：记边，并把标签条翻到底部（VS 工具窗的页签位置）。
  /// fresh=true 表示这栏是这一下动作刚劈出来的：库劈中心区时把权重对半分（1000→500/500），
  /// 新宿主白占半个窗口，看着就像布局压根没记住。按记下的份额（没有就按该边出厂比例）重配。
  const adoptDockTabset = (ts: TabSetNode, side: DockSide, fresh = false) => {
    dockedRef.current.set(ts.getId(), side);
    if (ts.getTabLocation() !== "bottom") {
      layoutAction(Actions.updateNodeAttributes(ts.getId(), { tabLocation: "bottom" }).setUserData(FROM_APP));
    }
    if (!fresh) return;
    const f = dockFracFor(side);
    const row = ts.getParent();
    const others = (row?.getChildren() ?? []).filter((c) => c !== ts);
    const sumOther = others.reduce((a, c) => a + nodeWeight(c), 0);
    // 份额换成 weight 要和同行的兄弟节点同量纲：w = 兄弟之和 * f/(1-f)，拖过分隔条也不会算串台。
    // 不设数值下限：weight 只是相对值，劈几次之后绝对数会很小，按比例算照样对。
    const weight = sumOther > 0 ? Math.max(1, Math.round((sumOther * f) / (1 - f))) : BORDER_SIZE[side];
    layoutAction(
      Actions.updateNodeAttributes(ts.getId(), {
        weight,
        minWidth: BORDER_MIN[side],
        maxWidth: BORDER_MAX[side],
      }).setUserData(FROM_APP),
    );
    if (sumOther <= 0) restoreCenterWeight(model);
  };

  /// 某条边该占多厚：优先这条边上钉住过的面板记下的份额（用户拖过分隔条就是他的），
  /// 其次该边出厂的 BORDER_SIZE/CENTER_WEIGHT 配比——和 defaultLayout 里的比例是同一个数。
  const dockFracFor = (side: DockSide): number => {
    for (const st of Object.values(panelDock.current)) {
      if (st.side === side && !st.hidden && st.frac && st.frac > 0.02 && st.frac < 0.8) return st.frac;
    }
    return BORDER_SIZE[side] / (BORDER_SIZE[side] + CENTER_WEIGHT);
  };

  /// tabset 在哪条边。这个 flexlayout 版本 tabset 在 DOM 里没有节点 id（只有 data-layout-path="/tsN"），
  /// 所以分三步认：①默认边 dock 的 id 直接反查（最稳，DOM 没重画也对）；②用户拖出来新劈的 tabset
  /// 用它孩子（面板标签按钮 id）反查 DOM 元素量几何；③都不行退回 left。
  const sideOfTabset = (id: string): DockSide => {
    for (const [side, tid] of Object.entries(DOCK_TABSET) as [DockSide, string][]) {
      if (tid === id) return side;
    }
    const lay = document.querySelector(".flexlayout__layout");
    const node = model.getNodeById(id);
    let el: Element | null = null;
    if (node instanceof TabSetNode) {
      for (const c of node.getChildren()) {
        el = document.getElementById(`flexlayout-tabbutton-${c.getId()}`)?.closest(".flexlayout__tabset") ?? null;
        if (el) break;
      }
    }
    if (!el || !lay) return "left";
    const r = el.getBoundingClientRect();
    const l = lay.getBoundingClientRect();
    const nx = (r.x + r.width / 2 - l.x) / l.width;
    const ny = (r.y + r.height / 2 - l.y) / l.height;
    const edges: [DockSide, number][] = [["left", nx], ["right", 1 - nx], ["top", ny], ["bottom", 1 - ny]];
    edges.sort((a, b) => a[1] - b[1]);
    return edges[0][0];
  };

  /// 用户拖放这一下想去的边：MOVE_NODE 的 data.location 就是落点方位。新劈的 tabset DOM 还没重画，
  /// 量几何会读到旧位置，所以认领新宿主时优先信这个动作里的边。
  const dropEdge = (action: Action): DockSide | undefined => {
    if (action.type !== Actions.MOVE_NODE) return undefined;
    const name = (action.data as { location?: unknown }).location;
    return name === "left" || name === "right" || name === "top" || name === "bottom" ? name : undefined;
  };

  /// 按 store 把面板放到该在的地方：自动隐藏 -> 边条（收起不滑出）；钉住 -> 边 tabset（没有就劈一个）。
  /// 启动修旧存档、浮窗并回主区收游荡 tabset 都走它；已经在正确宿主里的面板原样不动。
  const placePanel = (m: Model, id: string) => {
    const node = m.getNodeById(id);
    if (!(node instanceof TabNode) || node.getLayoutId() !== Model.MAIN_LAYOUT_ID) return;
    const host = node.getParent();
    const st = dockState(id);
    if (st.hidden) {
      if (host instanceof BorderNode && host.getLocation().getName() === st.side) return;
      // select 必须 false：自动隐藏是收进边条，选中会立刻滑出盖住主区
      m.doAction(Actions.moveNode(id, borderId(st.side), DOCK_LOC[st.side], -1, false).setUserData(FROM_APP));
      return;
    }
    const ts = panelTabsetFor(st.side);
    if (host === ts) return;
    if (ts) {
      m.doAction(Actions.moveNode(id, ts.getId(), DockLocation.CENTER, -1, true).setUserData(FROM_APP));
    } else {
      m.doAction(Actions.moveNode(id, CENTER, DOCK_LOC[st.side], -1, true).setUserData(FROM_APP));
      const p = m.getNodeById(id)?.getParent();
      if (p instanceof TabSetNode) adoptDockTabset(p, st.side, true);
    }
  };

  /// 钉住 = 常驻（住在边 tabset，主区让位）；取消钉住 = 自动隐藏（收进边条，点页签才滑出，
  /// 点主区自动收起——库的 overlay 语义）。状态按面板记，同边可以一个钉住一个隐藏。
  /// 图钉画在面板自己标题栏的右上角（见 PanelPin），照 VS 的工具窗。
  const togglePin = (id: string) => {
    const st = dockState(id);
    rememberDock(id, st.side, !st.hidden, hostFrac(model, id));
    placePanel(model, id);
    // 面板内容走 TabContentRenderer(React.memo)，跨宿主搬完不会重画，图钉会停在旧状态
    layoutRef.current?.redraw();
  };

  /// 面板标题栏右上角的控件：钉住或自动隐藏都给出图钉（状态看宿主是 tabset 还是边条）。
  /// 浮起来时标题栏整行藏掉（照 VS：浮窗的名字和按钮都在窗口顶栏上），所以这里不给控件
  const headerCtl = (node: TabNode): HeaderCtl | undefined => {
    if (node.getLayoutId() !== Model.MAIN_LAYOUT_ID) return undefined;
    const host = node.getParent();
    if (host instanceof BorderNode) return { pinned: false, onTogglePin: () => togglePin(node.getId()) };
    if (host instanceof TabSetNode && host.getId() !== CENTER) return { pinned: true, onTogglePin: () => togglePin(node.getId()) };
    return undefined;
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
    locale,
    setLocale,
    toggleView,
    resetLayout: resetWorkspace,
    toggleTheme: () => setTheme((t) => (t === "dark" ? "light" : "dark")),
    newSession: () => setQuickConnect({ edit: undefined }),
    newFolder: addFolder,
    importConfig: () => setImportBox(true),
    mcp: () => setMcpBox(true),
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
            factory={panelContent}
            onModelChange={onModelChange}
            onRenderTab={onRenderTab}
            onContextMenu={onNodeContextMenu}
            supportsPopout={false}
            i18nTranslator={flTranslate}
            onTabSetPlaceHolder={(n) =>
              n.getId() === CENTER ? (
                <div className="empty-state">
                  <div className="empty-glyph">
                    <IconTerminal2 {...ink(22)} />
                  </div>
                  <div className="empty-title">{t("app.emptyTitle")}</div>
                  <div className="empty-desc">{t(treeHasSession(tree) ? "app.emptyDescHas" : "app.emptyDescNone")}</div>
                  <button className="btn btn-primary empty-cta" onClick={() => setQuickConnect({ edit: undefined })}>
                    <IconDeviceDesktopPlus {...ink(15)} />
                    {t("app.emptyNew")}
                  </button>
                </div>
              ) : null
            }
          />
          <DockCompass ref={compassRef} />
          <FloatCaptionControls onDockBack={dockBack} onClose={closeFloat} />
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
          title={quickConnect.edit ? t("qc.sessionPropsTitle", { name: quickConnect.edit.name }) : t("qc.defaultTitle")}
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

      {importBox && (
        <ImportDialog
          onCancel={() => setImportBox(false)}
          onPick={(src) => {
            setImportBox(false);
            void importSessions(src.cmd);
          }}
        />
      )}

      {mcpBox && <McpDialog onClose={() => setMcpBox(false)} />}

      {confirmBox && (
        <ConfirmDialog
          message={confirmBox.message}
          onCancel={() => setConfirmBox(null)}
          onConfirm={() => {
            const action = confirmBox.action;
            setConfirmBox(null);
            action();
          }}
        />
      )}

      {cmdProps && (
        <CommandProperties
          key={cmdProps.node?.id ?? "new"}
          title={cmdProps.node ? t("cmd.propsTitle", { name: cmdProps.node.name }) : t("cmd.newTitle")}
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

      {view.statusBar && <StatusBar info={statusInfo} tabCount={tabs.length} fontSize={fontSize} onOpenMcp={() => setMcpBox(true)} />}
      {about && <AboutDialog onClose={() => setAbout(false)} />}
    </div>
  );
}
