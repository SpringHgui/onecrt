import { useCallback, useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import PanelPin from "./PanelPin";
import { fmtRate, fmtSize } from "./format";
import { useI18n } from "./i18n";
import { ConnectParams } from "./QuickConnect";
import { enabledMethods } from "./types";
import "./perf.css";

/// 后端 stats.rs 的 OutEvent
type StatsEvent = { event: "ready" } | { event: "line"; data: string } | { event: "closed"; reason: string };

type Status = "idle" | "connecting" | "live" | "error";

/// 一条采样里的绝对计数，速率与百分比要等下一条才算得出
interface Raw {
  t: number;
  cpu: number[] | null;
  memTotal: number;
  /// MemAvailable（没有则退回 MemFree）
  memAvail: number | null;
  memFree: number | null;
  load: number[] | null;
  rx: number;
  tx: number;
  diskUsed: number;
  diskTotal: number;
  uptime: number;
}

/// 换算完差值的数据点，一条采样一个
interface Point {
  t: number;
  cpu: number | null;
  memPct: number | null;
  memUsed: number;
  memTotal: number;
  rx: number | null;
  tx: number | null;
  diskPct: number | null;
  diskUsed: number;
  diskTotal: number;
  load: number[] | null;
  uptime: number;
}

/// 曲线保留的采样数：2 秒间隔下约 3 分钟
const HISTORY = 90;
const INTERVALS = [1, 2, 5, 10];
const INTERVAL_STORE = "shellx.perfInterval";
/// exec 通道连上却一行不回来说明脚本没跑成（非 Linux、无 /proc 或登录 shell 不配合）
const FIRST_LINE_MS = 8000;

const RE_CPU = /^cpu\s+[\d ]+$/;
const RE_UPTIME = /^[\d.]+ [\d.]+$/;
const RE_LOAD = /^[\d.]+ [\d.]+ [\d.]+ /;
const RE_MEM = /^Mem\w+:\s+(\d+)/;
/// 只认前四列数字：最后一列挂载点可能带空格，不能拿它当行尾锚点。表头没有这组数字，不会误判
const RE_DF_ROW = /^\S+\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)%/;
/// 网卡名本身可能含冒号，数字列从最后一个冒号后面开始认
const RE_NET = /^(.*):([\d ][\d ]*)$/;

function nums(s: string): number[] {
  const out: number[] = [];
  for (const p of s.trim().split(/\s+/)) {
    const n = Number(p);
    if (Number.isFinite(n)) out.push(n);
  }
  return out;
}

/// 后端把 /proc 的原始片段用 ~ 拼成一行回传，这里按每段的形状归类
function parseLine(line: string, now: number): Raw {
  const raw: Raw = {
    t: now,
    cpu: null,
    memTotal: 0,
    memAvail: null,
    memFree: null,
    load: null,
    rx: 0,
    tx: 0,
    diskUsed: 0,
    diskTotal: 0,
    uptime: 0,
  };
  const ifaces: { name: string; rx: number; tx: number }[] = [];
  for (const seg of line.split("~")) {
    if (!seg) continue;
    const net = RE_NET.exec(seg);
    const df = RE_DF_ROW.exec(seg);
    if (RE_CPU.test(seg)) raw.cpu = nums(seg.slice(3));
    else if (RE_MEM.test(seg)) {
      const kb = Number(RE_MEM.exec(seg)![1]);
      if (seg.startsWith("MemTotal:")) raw.memTotal = kb;
      else if (seg.startsWith("MemFree:")) raw.memFree = kb;
      else if (seg.startsWith("MemAvailable:")) raw.memAvail = kb;
    } else if (RE_LOAD.test(seg)) raw.load = nums(seg).slice(0, 3);
    else if (df) {
      raw.diskTotal = Number(df[1]);
      raw.diskUsed = Number(df[2]);
    } else if (net) {
      const name = net[1].trim();
      const f = nums(net[2]);
      if (name && name !== "lo") ifaces.push({ name, rx: f[0] ?? 0, tx: f[8] ?? 0 });
    } else if (RE_UPTIME.test(seg)) raw.uptime = Number(seg.split(" ")[0]) || 0;
  }
  /// 只算物理口：k8s/容器节点上 cali*、veth*、tunl0、docker0 会把同一份流量重复计一遍
  const phys = ifaces.filter((i) => /^(eth|en|wl|bond|ib)/.test(i.name));
  for (const i of phys.length ? phys : ifaces) {
    raw.rx += i.rx;
    raw.tx += i.tx;
  }
  return raw;
}

function cpuPct(a: number[], b: number[]): number | null {
  const dTotal = b.reduce((s, n) => s + n, 0) - a.reduce((s, n) => s + n, 0);
  if (dTotal <= 0) return null;
  const idle = (x: number[]) => (x[3] ?? 0) + (x[4] ?? 0);
  return clampPct((1 - (idle(b) - idle(a)) / dTotal) * 100);
}

function clampPct(v: number): number | null {
  return Number.isFinite(v) ? Math.min(100, Math.max(0, v)) : null;
}

/// 计数器被重置（重启或回绕）时差值为负，这格按无数据画
function rate(a: number, b: number, dt: number): number | null {
  const d = b - a;
  return d < 0 || dt <= 0 ? null : d / dt;
}

function memPct(raw: Raw): number | null {
  const avail = raw.memAvail ?? raw.memFree;
  if (!raw.memTotal || avail === undefined || avail === null) return null;
  return clampPct(((raw.memTotal - avail) / raw.memTotal) * 100);
}

/// 轴的上界取不小于峰值的 1/2/5×10ⁿ，刻度才不会每秒跳一个数
function niceMax(peak: number): number {
  if (peak <= 0) return 1;
  const p = Math.pow(10, Math.floor(Math.log10(peak)));
  for (const m of [1, 2, 5, 10]) if (m * p >= peak) return m * p;
  return 10 * p;
}

function fmtUptime(sec: number): string {
  const d = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

interface Series {
  v: (number | null)[];
  color: string;
  fill?: boolean;
}

/// 迷你折线：viewBox 横向拉满，线条宽度用 non-scaling-stroke 保持 1 物理像素档
function Spark({ series, max }: { series: Series[]; max: number }) {
  const len = Math.max(...series.map((s) => s.v.length), 0);
  if (len < 2) return <div className="perf-spark" />;
  const H = 40;
  const PAD = 3;
  const xy = (v: number, i: number) =>
    `${((i / (len - 1)) * 100).toFixed(2)},${(H - PAD - (Math.min(Math.max(v, 0), max) / max) * (H - 2 * PAD)).toFixed(2)}`;
  return (
    <svg className="perf-spark" viewBox={`0 0 100 ${H}`} preserveAspectRatio="none" aria-hidden="true">
      {series.map((s, k) => {
        const pts = s.v.map((v, i) => (v === null || !Number.isFinite(v) ? null : xy(v, i))).filter(Boolean).join(" ");
        if (!pts) return null;
        return (
          <g key={k}>
            {s.fill && <polygon points={`0,${H} ${pts} 100,${H}`} fill={s.color} opacity="0.13" />}
            <polyline points={pts} fill="none" stroke={s.color} strokeWidth="1.3" vectorEffect="non-scaling-stroke" />
          </g>
        );
      })}
    </svg>
  );
}

interface Props {
  tabId: string | null;
  /// 采样连的是活动标签的目标机，与终端各连各的
  params: ConnectParams | null;
  onClose(): void;
  /// 顶栏右上角的图钉：钉住=常驻，取消=自动隐藏
  pinned?: boolean;
  onTogglePin?(): void;
  /// 顶栏作为拖拽把手（flexlayout 拖放），由 App 注入
  drag?: React.HTMLAttributes<HTMLDivElement>;
}

export default function PerfPanel({ tabId, params, onClose, pinned, onTogglePin, drag }: Props) {
  const { t } = useI18n();
  const [interval, setIntervalSec] = useState(() => {
    const n = Number(localStorage.getItem(INTERVAL_STORE));
    return INTERVALS.includes(n) ? n : 2;
  });
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);
  const [points, setPoints] = useState<Point[]>([]);
  /// 上一条原始采样：算差值用，不参与渲染
  const prevRaw = useRef<Raw | null>(null);

  const pickInterval = (sec: number) => {
    setIntervalSec(sec);
    localStorage.setItem(INTERVAL_STORE, String(sec));
  };

  const onLine = useCallback((line: string) => {
    const now = Date.now();
    const raw = parseLine(line, now);
    const p0 = prevRaw.current;
    prevRaw.current = raw;
    const dt = p0 ? (now - p0.t) / 1000 : 0;
    const pt: Point = {
      t: now,
      cpu: p0?.cpu && raw.cpu && dt > 0 ? cpuPct(p0.cpu, raw.cpu) : null,
      memPct: memPct(raw),
      memUsed: raw.memTotal - (raw.memAvail ?? raw.memFree ?? 0),
      memTotal: raw.memTotal,
      rx: p0 && dt > 0 ? rate(p0.rx, raw.rx, dt) : null,
      tx: p0 && dt > 0 ? rate(p0.tx, raw.tx, dt) : null,
      diskPct: raw.diskTotal ? clampPct((raw.diskUsed / raw.diskTotal) * 100) : null,
      diskUsed: raw.diskUsed,
      diskTotal: raw.diskTotal,
      load: raw.load,
      uptime: raw.uptime,
    };
    setPoints((h) => [...h.slice(-(HISTORY - 1)), pt]);
  }, []);

  /// 换标签页 = 换一台目标机；面板关掉时采样通道跟着关
  useEffect(() => {
    if (!tabId || !params) {
      setStatus("idle");
      setPoints([]);
      setError(null);
      return;
    }
    let gone = false;
    let gotLine = false;
    setStatus("connecting");
    setPoints([]);
    setError(null);
    prevRaw.current = null;

    const ch = new Channel<StatsEvent>();
    ch.onmessage = (ev) => {
      if (gone) return;
      if (ev.event === "line") {
        gotLine = true;
        setStatus("live");
        onLine(ev.data);
      } else if (ev.event === "closed") {
        setStatus("error");
        setError(ev.reason);
      }
    };
    invoke("stats_start", {
      args: {
        id: tabId,
        host: params.host,
        port: params.port,
        username: params.username,
        password: params.password,
        keyPath: params.keyPath,
        methods: enabledMethods(params.auth),
        interval,
      },
      onEvent: ch,
    }).catch((e) => {
      if (gone) return;
      setStatus("error");
      setError(String(e));
    });

    const timer = setTimeout(() => {
      if (!gone && !gotLine) {
        setStatus("error");
        setError(t("perf.noData"));
      }
    }, FIRST_LINE_MS);

    const id = tabId;
    return () => {
      gone = true;
      clearTimeout(timer);
      invoke("stats_stop", { id }).catch(() => {});
    };
  }, [tabId, params, interval, onLine, t]);

  const last = points[points.length - 1] ?? null;
  const netPeak = niceMax(Math.max(1, ...points.flatMap((p) => [p.rx ?? 0, p.tx ?? 0])));
  const host = params ? `${params.username}@${params.host}` : "";

  const row = (
    name: string,
    value: string,
    sub: string,
    series: Series[],
    max: number,
    maxLabel?: string
  ) => (
    <div className="perf-row">
      <div className="perf-row-head">
        <span className="perf-name">{name}</span>
        <span className="perf-val">{value}</span>
        <span className="perf-sub">{maxLabel ? `${sub} · ${maxLabel}` : sub}</span>
      </div>
      <Spark series={series} max={max} />
    </div>
  );

  const col = (k: "cpu" | "memPct" | "diskPct" | "rx" | "tx") => points.map((p) => p[k]);

  return (
    <div className="perfpanel">
      <div className="perf-bar" {...drag}>
        <span className="perf-title">{t("perf.title")}</span>
        <span className="perf-host" title={host}>
          {host || "—"}
        </span>
        <span className="perf-spacer" />
        <select className="perf-interval" value={interval} onChange={(e) => pickInterval(Number(e.target.value))} title={t("perf.intervalTip")}>
          {INTERVALS.map((s) => (
            <option key={s} value={s}>
              {t("perf.intervalOpt", { s })}
            </option>
          ))}
        </select>
        {onTogglePin && <PanelPin pinned={!!pinned} onToggle={onTogglePin} />}
        <button title={t("perf.close")} onClick={onClose}>
          <IconX {...ink(16)} />
        </button>
      </div>

      <div className="perf-body">
        {status === "idle" && <p className="perf-hint">{t("perf.noTab")}</p>}
        {status === "connecting" && <p className="perf-hint">{t("perf.connecting")}</p>}
        {status === "error" && <p className="perf-hint perf-err">{error ?? t("perf.failed")}</p>}
        {status === "live" && !last && <p className="perf-hint">{t("perf.waiting")}</p>}

        {last &&
          row(
            t("perf.cpu"),
            last.cpu === null ? "—" : `${last.cpu.toFixed(1)}%`,
            last.load ? `${t("perf.loadTip")} ${last.load.map((x) => x.toFixed(2)).join(" ")}` : "—",
            [{ v: col("cpu"), color: "var(--accent)", fill: true }],
            100
          )}
        {last &&
          row(
            t("perf.mem"),
            last.memPct === null ? "—" : `${last.memPct.toFixed(0)}%`,
            last.memTotal ? `${fmtSize(last.memTotal * 1024)} · ${t("perf.memUsedShort")} ${fmtSize(last.memUsed * 1024)}` : "—",
            [{ v: col("memPct"), color: "var(--ansi-yellow)", fill: true }],
            100
          )}
        {last &&
          row(
            t("perf.net"),
            `${fmtRate(last.rx ?? 0)} / ${fmtRate(last.tx ?? 0)}`,
            t("perf.netSub"),
            [
              { v: col("rx"), color: "var(--accent)" },
              { v: col("tx"), color: "var(--text-2)" },
            ],
            netPeak,
            `${t("perf.peak")} ${fmtRate(netPeak)}`
          )}
        {last &&
          row(
            t("perf.disk"),
            last.diskPct === null ? "—" : `${last.diskPct.toFixed(0)}%`,
            last.diskTotal ? `${t("perf.diskFree")} ${fmtSize((last.diskTotal - last.diskUsed) * 1024)}` : "—",
            [{ v: col("diskPct"), color: "var(--text-2)", fill: true }],
            100
          )}
      </div>

      <div className="perf-foot">
        <span>{t("perf.footSource")}</span>
        <span className="perf-spacer" />
        <span>{last ? `${t("perf.up")} ${fmtUptime(last.uptime)} · ${points.length} ${t("perf.samples")}` : ""}</span>
      </div>
    </div>
  );
}
