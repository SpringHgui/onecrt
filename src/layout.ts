import type { IJsonModel } from "flexlayout-react";

/// 中心 tabset 的固定 id：没有标签页时也要留住它，新连接才知道往哪插
export const CENTER = "center";

/// 三个工具窗口的 component 与节点 id 同名，菜单开关据此找节点
export const PANEL_SESSIONS = "sessions";
export const PANEL_COMMANDS = "commands";
export const PANEL_FILES = "filePanel";

const STORE = "shellx.layout";

export function defaultLayout(): IJsonModel {
  return {
    global: {
      // 原生窗口弹出要 Tauri 侧的 popout 载体，先只开放应用内浮动面板
      tabEnablePopout: false,
      tabEnablePopoutIcon: false,
      tabEnablePin: false,
      // 应用内浮动面板（可自由摆位）开放；三个工具窗口各自 enableFloat:false 保持贴边
      tabEnableFloat: true,
      tabEnableFloatIcon: true,
      tabSetEnableDeleteWhenEmpty: false,
      borderEnableDrop: true,
      borderSize: 320,
    },
    layout: {
      type: "row",
      children: [{ type: "tabset", id: CENTER, weight: 1, children: [] }],
    },
    borders: [
      {
        type: "border",
        location: "left",
        borderType: "overlay",
        size: 320,
        minSize: 220,
        maxSize: 560,
        selected: -1,
        children: [
          {
            type: "tab",
            id: PANEL_SESSIONS,
            name: "Session Manager",
            component: PANEL_SESSIONS,
            enableClose: false,
            enableFloat: false,
          },
          {
            type: "tab",
            id: PANEL_COMMANDS,
            name: "Command Manager",
            component: PANEL_COMMANDS,
            enableClose: false,
            enableFloat: false,
          },
        ],
      },
      {
        type: "border",
        location: "right",
        borderType: "overlay",
        size: 460,
        minSize: 280,
        maxSize: 760,
        selected: -1,
        children: [
          {
            type: "tab",
            id: PANEL_FILES,
            name: "File Panel",
            component: PANEL_FILES,
            enableClose: false,
            enableFloat: false,
          },
        ],
      },
    ],
  };
}

type JsonNode = { type?: string; component?: string; children?: JsonNode[] };

function hasTab(node?: JsonNode): boolean {
  if (!node) return false;
  if (node.type === "tab") return true;
  return (node.children ?? []).some(hasTab);
}

/// 只留几何：重启时连接并不存在，终端标签必须丢掉，否则中心区域会挂着一排打不开的死标签。
/// subLayouts（浮动面板）里存的是完整的一份 model，所以要递归进去；剪空的浮动面板整个丢掉，
/// 不然重启后屏幕上会留一个空浮窗。
function pruneTerminals(json: unknown): void {
  const model = json as {
    layout?: JsonNode;
    borders?: JsonNode[];
    subLayouts?: Record<string, { layout?: JsonNode; borders?: JsonNode[] }>;
  };
  if (!model || typeof model !== "object") return;
  const walk = (node?: JsonNode): JsonNode | undefined => {
    if (!node || !Array.isArray(node.children)) return node;
    node.children = node.children.filter((c) => c.component !== "terminal").map((c) => walk(c) as JsonNode);
    return node;
  };
  walk(model.layout);
  model.borders?.forEach(walk);
  for (const [id, sub] of Object.entries(model.subLayouts ?? {})) {
    pruneTerminals(sub);
    if (!hasTab(sub.layout) && !(sub.borders ?? []).some(hasTab)) delete model.subLayouts?.[id];
  }
}

export function loadLayout(): IJsonModel {
  try {
    const raw = localStorage.getItem(STORE);
    if (raw) {
      const json = JSON.parse(raw) as IJsonModel;
      pruneTerminals(json);
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
  } catch (e) {
    console.error(STORE, e);
  }
}
