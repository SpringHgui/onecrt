import "./menu.css";

export type TabStatus = "connecting" | "ready" | "error" | "closed";

export interface StatusInfo {
  name: string;
  status: TabStatus;
  host: string;
  port: number;
  username: string;
  cols: number;
  rows: number;
}

const STATUS_TEXT: Record<TabStatus, string> = {
  connecting: "连接中",
  ready: "已连接",
  error: "连接失败",
  closed: "已断开",
};

interface Props {
  info: StatusInfo | null;
  tabCount: number;
  fontSize: number;
}

export default function StatusBar({ info, tabCount, fontSize }: Props) {
  return (
    <div className="statusbar">
      {info ? (
        <>
          <span className={`status-dot ${info.status}`} />
          <span className="sb-name">{info.name}</span>
          <span className="sb-sep" />
          <span>
            {info.username ? `${info.username}@` : ""}
            {info.host}:{info.port}
          </span>
          <span className="sb-sep" />
          <span>{STATUS_TEXT[info.status]}</span>
          <span className="sb-fill" />
          <span>
            {info.cols} × {info.rows}
          </span>
        </>
      ) : (
        <>
          <span>无活动会话</span>
          <span className="sb-fill" />
        </>
      )}
      <span className="sb-sep" />
      <span>{tabCount} 个标签</span>
      <span className="sb-sep" />
      <span>{fontSize}px</span>
    </div>
  );
}
