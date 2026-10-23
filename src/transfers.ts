import { useSyncExternalStore } from "react";

/// 传输记录的单一来源：文件面板写，底部传输面板和状态栏读。
/// 放在组件树外面是因为读它的两处（右侧文件面板 / 底部状态栏）不在同一条父子链上。

export type TransferDir = "up" | "down";
export type TransferStatus = "running" | "done" | "error";

export interface Transfer {
  id: number;
  dir: TransferDir;
  name: string;
  done: number;
  total: number;
  /// 字节/秒，由相邻两次进度采样算出
  speed: number;
  status: TransferStatus;
  /// 失败原因，成功时为空串
  error: string;
  startedAt: number;
  /// 0 表示还没结束
  endedAt: number;
}

/// 后端每 64KB 发一次 tick，1GB 的文件就是上万次；合并到 ~12 次/秒，UI 看不出差
const TICK_MS = 80;

let items: Transfer[] = [];
/// 交给 React 的不可变快照，只在发布时换引用
let snapshot: Transfer[] = items;
/// 有没有传输正在跑；单独存一份，让只关心这个布尔量的订阅者不被进度 tick 带着重渲染
let activity = false;
const listeners = new Set<() => void>();
/// 每个传输的上一次采样点，用来算速率
const samples = new Map<number, { at: number; done: number }>();
let nextId = 1;

function publish() {
  snapshot = items;
  activity = items.some((t) => t.status === "running");
  for (const l of listeners) l();
}

function patch(id: number, f: (t: Transfer) => Transfer) {
  const i = items.findIndex((t) => t.id === id);
  if (i < 0) return;
  const next = items.slice();
  next[i] = f(next[i]);
  items = next;
  publish();
}

export function beginTransfer(dir: TransferDir, name: string, total: number): number {
  const id = nextId++;
  const now = Date.now();
  items = [
    ...items,
    { id, dir, name, done: 0, total, speed: 0, status: "running", error: "", startedAt: now, endedAt: 0 },
  ];
  samples.set(id, { at: now, done: 0 });
  publish();
  return id;
}

export function reportTransfer(id: number, done: number, total: number) {
  const prev = samples.get(id);
  if (!prev) return;
  const now = Date.now();
  const dt = now - prev.at;
  if (dt < TICK_MS) return;
  const inst = ((done - prev.done) * 1000) / dt;
  samples.set(id, { at: now, done });
  patch(id, (t) => ({
    ...t,
    done,
    total: total || t.total,
    // 瞬时速率抖得厉害，留一点旧值压一压
    speed: t.speed ? t.speed * 0.6 + inst * 0.4 : inst,
  }));
}

export function endTransfer(id: number, error?: string) {
  samples.delete(id);
  patch(id, (t) => ({
    ...t,
    status: error ? "error" : "done",
    error: error ?? "",
    speed: 0,
    endedAt: Date.now(),
    // 最后一段 tick 可能被合并掉，成功就按传完算
    done: error || !t.total ? t.done : t.total,
  }));
}

export function clearFinished() {
  const next = items.filter((t) => t.status === "running");
  if (next.length === items.length) return;
  items = next;
  publish();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useTransfers(): Transfer[] {
  return useSyncExternalStore(subscribe, () => snapshot);
}

/// 文件面板只用它来禁用上传/下载：进度 tick 不该让整份文件列表跟着重画
export function useTransferActivity(): boolean {
  return useSyncExternalStore(subscribe, () => activity);
}

/// 同一时刻只跑一个传输（文件面板在传输中禁用上传/下载），取最后一个即可
export function runningTransfer(list: Transfer[]): Transfer | null {
  for (let i = list.length - 1; i >= 0; i--) if (list[i].status === "running") return list[i];
  return null;
}

export function pctOf(t: Transfer): number {
  return t.total ? Math.min(100, Math.round((t.done / t.total) * 100)) : 0;
}
