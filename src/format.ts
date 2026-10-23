/// 大小 / 速率 / 耗时的显示口径：文件面板、传输面板、状态栏共用一份

export function fmtSize(n: number | null): string {
  if (n === null) return "—";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

export function fmtRate(bytesPerSec: number): string {
  if (bytesPerSec < 1) return "—";
  return `${fmtSize(Math.round(bytesPerSec))}/s`;
}

export function fmtElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const p = (n: number) => String(n).padStart(2, "0");
  if (s < 60) return `00:${p(s)}`;
  if (s < 3600) return `${p(Math.floor(s / 60))}:${p(s % 60)}`;
  return `${Math.floor(s / 3600)}:${p(Math.floor((s % 3600) / 60))}:${p(s % 60)}`;
}
