import type { IJsonModel, IJsonRowNode, IJsonTabNode, IJsonTabSetNode } from "flexlayout-react";
import { t } from "./i18n";

/// 中心 tabset 的固定 id：没有标签页时也要留住它，新连接才知道往哪插
export const CENTER = "center";

/// 工具窗口的 component 与节点 id 同名，菜单开关据此找节点
export const PANEL_SESSIONS = "sessions";
export const PANEL_COMMANDS = "commands";
export const PANEL_FILES = "filePanel";
export const PANEL_TRANSFERS = "transfers";
export const PANEL_PERF = "perf";

const STORE = "shellx.layout";
/// 面板被视图菜单关掉后，它最后停靠的那条边记在这里，下次打开还原到原处
const DOCK_STORE = "shellx.panelDock";

export type DockSide = "left" | "right" | "top" | "bottom";

/// 四条边都是可停靠的工具窗区。空边靠 enableAutoHide 收起来不占位，
/// 拖到那条边附近时库会临时把它露出来（checkForBorderToShow）。
/// 边条只承担 VS 的"自动隐藏"：钉住的面板不住在边里，住在主布局边缘的 tabset 里（见 DOCK_TABSET）。
const BORDER_SIDES: DockSide[] = ["left", "right", "top", "bottom"];

/// 钉住（常驻）面板的宿主 tabset id：每边一个，住多个面板时就是 VS 的选项卡组。
/// 标签条翻到底部（tabLocation:bottom），照 VS 工具窗的页签位置。
export const DOCK_TABSET: Record<DockSide, string> = {
  left: "dock_left",
  right: "dock_right",
  top: "dock_top",
  bottom: "dock_bottom",
};

/// 各边的默认厚度：左右放树状面板要宽，上下放横向内容要矮。
/// 钉住态用 tabset 的 weight 按同一组数值配比初始宽度；宿主被扫掉后重钉也按这组数复原。
export const BORDER_SIZE: Record<DockSide, number> = { left: 320, right: 460, top: 240, bottom: 176 };
export const BORDER_MIN: Record<DockSide, number> = { left: 220, right: 280, top: 140, bottom: 120 };
export const BORDER_MAX: Record<DockSide, number> = { left: 560, right: 760, top: 480, bottom: 520 };

/// 中心区在根 row 里的权重。和 dock 的 weight(=BORDER_SIZE) 竞争份额的是"装着 CENTER 的最外层
/// 容器"那一格，不是 CENTER 自己——库的 row weight 默认才 100，中心区不显式写就会被 320/460 压成窄条，
/// 劈一栏之后每边只有一百来像素，看着像被隐藏了。
export const CENTER_WEIGHT = 1000;

export function borderId(side: DockSide): string {
  return `border_${side}`;
}

/// 面板页签的显示名走 i18n：布局 JSON 里的 name 是会被存盘的，所以出厂名按当前语言生成，
/// 切语言时由 App 里的 syncPanelTabNames 换名（传输区已并进文件面板，只服务旧存档）。
export const PANEL_TAB_KEY: Record<string, string> = {
  [PANEL_SESSIONS]: "sm.title",
  [PANEL_COMMANDS]: "cm.title",
  [PANEL_FILES]: "fp.title",
  [PANEL_PERF]: "perf.title",
  [PANEL_TRANSFERS]: "tp.title",
};

const panelName = (id: string): string => (PANEL_TAB_KEY[id] ? t(PANEL_TAB_KEY[id]) : id);

const panelTab = (id: string): IJsonTabNode => ({
  type: "tab",
  id,
  name: panelName(id),
  component: id,
  enableClose: false,
  enableFloat: false,
});

/// 面板的初始归属边；其余边留空，等用户拖过去。
/// 函数不是常量：页签名要按调用时的语言现取，冻成常量会把名字停在模块求值那一刻。
const defaultChildren = (): Record<DockSide, IJsonTabNode[]> => ({
  left: [panelTab(PANEL_SESSIONS), panelTab(PANEL_COMMANDS)],
  right: [panelTab(PANEL_FILES), panelTab(PANEL_PERF)],
  top: [],
  /// 传输区不再是独立工具窗：它固定在文件面板底部（见 FilePanel），四条边出厂都是空的
  bottom: [],
});

/// 面板标签的出厂定义（显示名、不许直接关标签、不许浮出）。视图菜单重开面板要按 id 取回这一份：
/// 少了 enableClose:false 用户就能直接关掉面板标签，而视图开关不知道这件事（它只在开关时同步布局），
/// 结果是"菜单里还勾着、面板却没了"；名字退成 id 则让浮窗顶栏写出 "commands" 这种内部名。
export function panelTabJson(id: string): IJsonTabNode {
  const known = Object.values(defaultChildren()).some((tabs) => tabs.some((t) => t.id === id));
  return known ? panelTab(id) : { type: "tab", id, name: panelName(id), component: id };
}

/// 钉住态的宿主 tabset：标签条在底部（VS 工具窗页签位置），minWidth 沿用边条的最小厚度
const dockTabset = (side: DockSide, extra?: Partial<IJsonTabSetNode>): IJsonTabSetNode => ({
  type: "tabset",
  id: DOCK_TABSET[side],
  tabLocation: "bottom",
  weight: BORDER_SIZE[side],
  minWidth: BORDER_MIN[side],
  children: defaultChildren()[side].map((c) => ({ ...c })),
  ...extra,
});

export function defaultLayout(): IJsonModel {
  return {
    global: {
      // 原生窗口弹出要 Tauri 侧的 popout 载体，先只开放应用内浮动面板
      tabEnablePopout: false,
      tabEnablePopoutIcon: false,
      tabEnablePin: false,
      // 应用内浮动面板（可自由摆位）开放；工具窗口各自 enableFloat:false 保持贴边
      tabEnableFloat: true,
      tabEnableFloatIcon: true,
      tabSetEnableDeleteWhenEmpty: false,
      borderEnableDrop: true,
      borderSize: 320,
      // 收进边条的面板保持收起：落条=隐藏，点页签才滑出（VS 的自动隐藏）
      borderAutoSelectTabWhenOpen: false,
      borderAutoSelectTabWhenClosed: false,
    },
    layout: {
      type: "row",
      children: [
        dockTabset("left"),
        {
          // 库没有 col：嵌套的 row 自动换向。这层"中心区格"出厂只装 CENTER，用户往上下左右拖出钉住
          // 面板时新栏就劈在这里；它的 weight 才是中心区在根 row 里争取到的份额，必须显式给，
          // 缺省 100 会被边 dock 压扁。
          type: "row",
          weight: CENTER_WEIGHT,
          children: [{ type: "tabset", id: CENTER, weight: CENTER_WEIGHT, children: [] }],
        },
        dockTabset("right"),
      ],
    },
    borders: BORDER_SIDES.map((side) => ({
      type: "border" as const,
      location: side,
      borderType: "overlay",
      size: BORDER_SIZE[side],
      minSize: BORDER_MIN[side],
      maxSize: BORDER_MAX[side],
      selected: -1,
      // 空边不占位：拖到该边附近时库会临时露出它来接住落点
      enableAutoHide: true,
      children: [],
    })),
  };
}

type JsonNode = { type?: string; component?: string; children?: JsonNode[] };

function hasTab(node?: JsonNode): boolean {
  if (!node) return false;
  if (node.type === "tab") return true;
  return (node.children ?? []).some(hasTab);
}

/// 只留几何：重启时连接并不存在，终端标签必须丢掉，否则中心区域会挂着一排打不开的死标签。
/// 老存档里的独立传输面板也一并丢掉：它现在是文件面板底部的固定区，留着会在启动时留一个空宿主栏。
/// subLayouts（浮动面板）里存的是完整的一份 model，所以要递归进去；剪空的浮动面板整个丢掉，
/// 不然重启后屏幕上会留一个空浮窗。
function pruneGoneTabs(json: unknown): void {
  const model = json as {
    layout?: JsonNode;
    borders?: JsonNode[];
    subLayouts?: Record<string, { layout?: JsonNode; borders?: JsonNode[] }>;
  };
  if (!model || typeof model !== "object") return;
  const gone = new Set(["terminal", PANEL_TRANSFERS]);
  const walk = (node?: JsonNode): JsonNode | undefined => {
    if (!node || !Array.isArray(node.children)) return node;
    node.children = node.children.filter((c) => !gone.has(String(c.component))).map((c) => walk(c) as JsonNode);
    return node;
  };
  walk(model.layout);
  model.borders?.forEach(walk);
  for (const [id, sub] of Object.entries(model.subLayouts ?? {})) {
    pruneGoneTabs(sub);
    if (!hasTab(sub.layout) && !(sub.borders ?? []).some(hasTab)) delete model.subLayouts?.[id];
  }
}

export function loadLayout(): IJsonModel {
  try {
    const raw = localStorage.getItem(STORE);
    if (raw) {
      const json = JSON.parse(raw) as IJsonModel;
      pruneGoneTabs(json);
      ensureBorders(json);
      normalizeBorders(json);
      raiseCenterRegion(json);
      // 老存档没有这两个全局键时库默认 true：面板落进边条会立刻滑出盖住主区，
      // 自动隐藏就该保持收起，强制关掉。
      json.global = { ...json.global, borderAutoSelectTabWhenOpen: false, borderAutoSelectTabWhenClosed: false };
      // 自动隐藏的面板重启后一律收起：overlay 的"开着"是瞬态，落盘时带出来会在启动时盖住主区。
      // borderType 等于库默认值(split)时 toJson 会省掉这个键，缺失要按 split 算。
      for (const b of json.borders ?? []) {
        if ((b.borderType ?? "split") !== "split") b.selected = -1;
      }
      return json;
    }
  } catch (e) {
    console.error(STORE, e);
  }
  return defaultLayout();
}

/// 老存档只有左右两条边。缺的边补成空的自动隐藏边，四条边才都能接住拖过去的面板。
function ensureBorders(json: IJsonModel): void {
  const borders = (json.borders ??= []);
  const have = new Set(borders.map((b) => b.location));
  for (const def of defaultLayout().borders ?? []) {
    if (!have.has(def.location)) borders.push(def);
  }
  // 库按声明顺序决定边的层级，保持左→右→上→下
  borders.sort((a, b) => BORDER_SIDES.indexOf(a.location as DockSide) - BORDER_SIDES.indexOf(b.location as DockSide));
}

/// 老存档的左右边是 split 且没开自动隐藏：空的时候不收，屏幕上留两条空白竖条。
/// 边只承担"自动隐藏"这一职（钉住的面板住在主布局 tabset 里），四条边统一成 overlay+autoHide，
/// 空边才不占位。已存在的边也要归一，ensureBorders 只补缺的边、不动旧边的属性。
function normalizeBorders(json: IJsonModel): void {
  for (const b of json.borders ?? []) {
    b.borderType = "overlay";
    b.enableAutoHide = true;
  }
}

/// 老存档里包着 CENTER 的那层嵌套 row 常常没写 weight，于是中心区按库默认 100 和 dock 的 320/460 竞争，
/// 只剩一成多的宽度；劈一栏后每边百来像素，看着像被隐藏了。从 CENTER 往上把缺 weight 的层补成中心区的量级。
/// 只补"没写过"的层：用户拖分隔条会落下显式数值，那种意图不覆盖。
function raiseCenterRegion(json: IJsonModel): void {
  const root = json.layout;
  if (!root) return;
  const trail: (IJsonRowNode | IJsonTabSetNode)[] = [];
  const path: (IJsonRowNode | IJsonTabSetNode)[] = [];
  const find = (n: IJsonRowNode | IJsonTabSetNode): boolean => {
    if (n.type === "tabset" && n.id === CENTER) {
      trail.push(...path, n);
      return true;
    }
    path.push(n);
    for (const c of (n.children ?? []) as (IJsonRowNode | IJsonTabSetNode)[]) {
      if (c.type !== "row" && c.type !== "tabset") continue;
      if (find(c)) return true;
    }
    path.pop();
    return false;
  };
  if (!find(root)) return;
  for (const n of trail.slice(1)) if (n.weight === undefined) n.weight = CENTER_WEIGHT;
}

/// ---------- 面板归属边、钉住状态与宿主厚度的记忆 ----------
/// 视图菜单关掉面板时节点会被删掉，重开要落回它原来那条边，而不是硬编码的默认边。
/// hidden=true 表示自动隐藏（收在边条里）；false 表示钉住（主布局边缘的 tabset）。
/// frac 是钉住时宿主在它那一行里的宽度份额（0~1）：整条边都拔成自动隐藏后宿主会被当空壳扫掉，
/// 重钉只能新劈一栏，没有这个数就只能回到出厂厚度。
/// 用份额不用 weight：分隔条拖过一次之后同一行的 weight 会被归一成百分比（24.9 对 75.1），
/// 和出厂那套 320/1000 不同量纲，直接存 weight 下次会把栏算没。
/// 老存档只记边（字符串），按钉住算：VS 的默认态就是 docked。
export type PanelDockState = { side: DockSide; hidden: boolean; frac?: number };

export function loadPanelDock(): Record<string, PanelDockState> {
  const out: Record<string, PanelDockState> = {};
  try {
    const raw = localStorage.getItem(DOCK_STORE);
    if (raw) {
      for (const [id, v] of Object.entries(JSON.parse(raw) as Record<string, unknown>)) {
        if (typeof v === "string") out[id] = { side: v as DockSide, hidden: false };
        else if (v && typeof v === "object") out[id] = { ...(v as PanelDockState) };
      }
    }
  } catch (e) {
    console.error(DOCK_STORE, e);
  }
  return out;
}

export function savePanelDock(map: Record<string, PanelDockState>) {
  try {
    localStorage.setItem(DOCK_STORE, JSON.stringify(map));
  } catch (e) {
    console.error(DOCK_STORE, e);
  }
}

export function saveLayout(json: IJsonModel) {
  try {
    localStorage.setItem(STORE, JSON.stringify(json));
  } catch (e) {
    console.error(STORE, e);
  }
}

export function clearLayout() {
  try {
    localStorage.removeItem(STORE);
    localStorage.removeItem(DOCK_STORE);
  } catch (e) {
    console.error(STORE, e);
  }
}
