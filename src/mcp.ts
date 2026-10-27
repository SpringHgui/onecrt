import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface McpAgent {
  terminalId: string;
  sessionName: string;
  host: string;
  closed: boolean;
  outBytes: number;
  lastCommand: string;
}

export interface McpStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  token: string;
  url: string;
  configFile: string;
  error: string | null;
  agents: McpAgent[];
}

/// 粘到 MCP 客户端（Qoder / Claude Desktop / Cursor）里的那段配置
export function snippet(s: McpStatus): string {
  return JSON.stringify(
    { mcpServers: { shellx: { type: "http", url: s.url, headers: { Authorization: `Bearer ${s.token}` } } } },
    null,
    2,
  );
}

/// 展示用的版本：令牌换成点号，免得悬浮卡片把令牌摊在屏幕上；复制仍走 snippet()
export function snippetMasked(s: McpStatus): string {
  return snippet(s).replace(s.token, "•".repeat(Math.min(s.token.length, 24)));
}

/// MCP 状态的唯一取数入口：状态栏常驻一格、悬浮卡片和弹窗都读它，
/// 免得三处各写一份轮询与开关调用后互相不一致。pollMs 由调用方按可见程度给。
export function useMcp(pollMs: number) {
  const [st, setSt] = useState<McpStatus | null>(null);
  const [err, setErr] = useState("");

  const refresh = useCallback(() => {
    invoke<McpStatus>("mcp_status").then(setSt, (e) => setErr(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, pollMs);
    return () => clearInterval(id);
  }, [refresh, pollMs]);

  /// 开关、改端口、换令牌都回一份新状态
  const call = useCallback((cmd: string, args?: Record<string, unknown>) => {
    setErr("");
    invoke<McpStatus>(cmd, args).then(setSt, (e) => setErr(String(e)));
  }, []);

  return { st, err, setErr, call, refresh };
}
