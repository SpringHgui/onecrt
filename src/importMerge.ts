import { isFolder, Node } from "./types";

/// 导入合并引擎：把后端解析出的一棵 imported 树并入现有会话树。
/// 三条硬约束（对应设计确认 1A / 2A / 3A）：
///   1A 冲突键 = host + port（同机同端口即视为同一条会话）；
///   2A 覆盖只改 password，其它字段（name / username / keyPath / auth / 所在文件夹 / id /
///      expanded）一律保留；且 imported 侧 password 为空（SecureCRT 设了主口令解不出、
///      WinSCP 一律留空）时**不改**，绝不用空覆盖已有口令。
///   3A 文件夹按名字在既有树里递归查找同名夹、进去继续 merge，找不到才新建；
///      expanded 用「OR」——任一侧展开就展开。
/// 现有树只读：所有函数返回新数组，不原地修改。

/** 1A：会话身份键。host 大小写与首尾空格在 SecureCRT 里可能漂，做一次归一。 */
export const sessionKey = (host: string, port: number): string =>
  `${host.trim().toLowerCase()}:${port}`;

/** 一条 imported 会话对现有树的覆盖描述。 */
export interface ConflictEntry {
  /** 命中的现有会话 id 列表（同 host+port 在现有树里出现多处时全部覆盖） */
  existingIds: string[];
  /** imported 侧解出的明文口令；空 → 不改写 */
  newPassword: string;
}

export interface ImportPlan {
  /** 只含"没匹配到现有"的会话（保留其外层文件夹结构）；经 applyAppend 并入 */
  toAppend: Node[];
  /** 冲突列表；长度 = 冲突会话数（不去重，同 host+port 在 imported 里出现两次就算两条） */
  conflicts: ConflictEntry[];
  newCount: number;
  conflictCount: number;
}

function indexByHostPort(nodes: Node[], out: Map<string, string[]>): void {
  for (const n of nodes) {
    if (isFolder(n)) indexByHostPort(n.children, out);
    else {
      const k = sessionKey(n.host, n.port);
      const arr = out.get(k);
      if (arr) arr.push(n.id);
      else out.set(k, [n.id]);
    }
  }
}

/** 从 imported 树里剔除"命中现有"的会话（收进 conflicts），保留新增；顺带裁掉变空的文件夹。 */
function prune(
  nodes: Node[],
  existing: Map<string, string[]>,
  conflicts: ConflictEntry[],
): Node[] {
  const kept: Node[] = [];
  for (const n of nodes) {
    if (isFolder(n)) {
      const kids = prune(n.children, existing, conflicts);
      if (kids.length > 0) kept.push({ ...n, children: kids });
    } else {
      const hit = existing.get(sessionKey(n.host, n.port));
      if (hit) conflicts.push({ existingIds: hit.slice(), newPassword: n.password });
      else kept.push(n);
    }
  }
  return kept;
}

/** 3A：把 incoming 并入 target——同名文件夹递归 merge，其它作为新兄弟追加。 */
function mergeFolders(target: Node[], incoming: Node[]): Node[] {
  const out = target.slice();
  for (const n of incoming) {
    if (isFolder(n)) {
      const idx = out.findIndex((t) => isFolder(t) && t.name === n.name);
      if (idx >= 0) {
        const t = out[idx];
        // 只有命中的确实是文件夹才 merge（findIndex 已保证，这里让类型收窄）
        if (isFolder(t)) {
          out[idx] = {
            ...t,
            children: mergeFolders(t.children, n.children),
            expanded: t.expanded || n.expanded,
          };
        }
      } else {
        out.push(n);
      }
    } else {
      out.push(n);
    }
  }
  return out;
}

function countSessions(nodes: Node[]): number {
  let n = 0;
  for (const x of nodes) n += isFolder(x) ? countSessions(x.children) : 1;
  return n;
}

/** 制定导入计划。existing 与 imported 都不被修改。 */
export function planImport(existing: Node[], imported: Node[]): ImportPlan {
  const idx = new Map<string, string[]>();
  indexByHostPort(existing, idx);
  const conflicts: ConflictEntry[] = [];
  const toAppend = prune(imported, idx, conflicts);
  return {
    toAppend,
    conflicts,
    newCount: countSessions(toAppend),
    conflictCount: conflicts.length,
  };
}

/** 3A：把 plan.toAppend 并入 existing，同名文件夹递归合并。 */
export function applyAppend(existing: Node[], toAppend: Node[]): Node[] {
  return mergeFolders(existing, toAppend);
}

/**
 * 2A：按 conflicts 覆盖现有会话的 password。
 * - 只动 password 字段；其它字段原样返回引用（浅拷贝只在真正改动的节点上发生）
 * - newPassword 为空则跳过（不用空覆盖有）
 * - 同一 id 被多条冲突命中时，最后一条生效（保留 imported 顺序里的"最新"）
 */
export function applyOverwrite(existing: Node[], conflicts: ConflictEntry[]): Node[] {
  const byId = new Map<string, string>();
  for (const c of conflicts) {
    if (!c.newPassword) continue;
    for (const id of c.existingIds) byId.set(id, c.newPassword);
  }
  if (byId.size === 0) return existing;
  const walk = (nodes: Node[]): Node[] =>
    nodes.map((n) => {
      if (isFolder(n)) return { ...n, children: walk(n.children) };
      const p = byId.get(n.id);
      return p === undefined ? n : { ...n, password: p };
    });
  return walk(existing);
}
