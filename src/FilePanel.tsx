import { useCallback, useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
  IconArrowBarUp,
  IconFile,
  IconFileDownload,
  IconFileUpload,
  IconFolder,
  IconFolderPlus,
  IconFolderStar,
  IconHome,
  IconPencil,
  IconRefresh,
  IconServer,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import { ink } from "./icons";
import InlineEdit from "./InlineEdit";
import { ConnectParams } from "./QuickConnect";
import "./filepanel.css";

interface Entry {
  name: string;
  path: string;
  dir: boolean;
  size: number | null;
  mtime: number | null;
}

interface Side {
  path: string;
  /// 本地侧的上级由后端算（盘符根的规则不该在 JS 再写一遍），远端用不到
  parent: string;
  entries: Entry[];
  sel: Entry | null;
}

type Prog = { name: string; done: number; total: number };
type ProgEvent =
  | { event: "tick"; name: string; done: number; total: number }
  | { event: "done"; name: string };

type RemoteState = "idle" | "connecting" | "ready";

const emptySide: Side = { path: "", parent: "", entries: [], sel: null };

/// renaming 的哨兵值：表示当前是"新建目录"那一行，而不是某个已存在条目在改名
const NEW_ROW = "\0new";

function fmtSize(n: number | null): string {
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

function fmtDate(sec: number | null): string {
  if (!sec) return "";
  const d = new Date(sec * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/// 两侧都用它拆面包屑：Windows 走盘符分支，SFTP 的 POSIX 路径走下半段
function crumbs(p: string): { label: string; path: string }[] {
  if (!p.trim()) return [];
  const drive = /^([A-Za-z]:)/.exec(p);
  if (drive) {
    const rest = p.slice(drive[1].length).split(/[\\/]/).filter(Boolean);
    return [
      { label: drive[1], path: `${drive[1]}\\` },
      ...rest.map((n, i) => ({ label: n, path: `${drive[1]}\\${rest.slice(0, i + 1).join("\\")}` })),
    ];
  }
  const parts = p.split("/").filter(Boolean);
  return [
    { label: "/", path: "/" },
    ...parts.map((n, i) => ({ label: n, path: `/${parts.slice(0, i + 1).join("/")}` })),
  ];
}

function remoteParent(p: string): string {
  const t = p.replace(/\/+$/, "");
  const i = t.lastIndexOf("/");
  return i <= 0 ? "/" : t.slice(0, i);
}

function remoteChild(p: string, name: string): string {
  return `${p.replace(/\/+$/, "")}/${name}`;
}

interface Props {
  tabId: string | null;
  params: ConnectParams | null;
  onClose(): void;
}

export default function FilePanel({ tabId, params, onClose }: Props) {
  const [local, setLocal] = useState<Side>(emptySide);
  const [remote, setRemote] = useState<Side>(emptySide);
  const [remoteState, setRemoteState] = useState<RemoteState>("idle");
  /// 新建/重命名/删除作用于哪一侧；点任一栏即切换
  const [focus, setFocus] = useState<"local" | "remote">("remote");
  const [prog, setProg] = useState<Prog | null>(null);
  const [error, setError] = useState<string | null>(null);
  /// 正在行内改名的条目路径，null 表示没有
  const [renaming, setRenaming] = useState<string | null>(null);
  /// 本地侧"快速访问"下拉（常用文件夹 + 盘符），null 表示没展开
  const [quick, setQuick] = useState<Entry[] | null>(null);
  const lseq = useRef(0);
  const rseq = useRef(0);

  const showErr = useCallback((e: unknown) => setError(String(e)), []);

  const toggleQuick = useCallback(() => {
    if (quick) {
      setQuick(null);
      return;
    }
    invoke<Entry[]>("local_shortcuts")
      .then((r) => setQuick(r.length ? r : null))
      .catch(showErr);
  }, [quick, showErr]);

  /// 点到菜单外面就收起来
  useEffect(() => {
    if (!quick) return;
    const onDown = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".fp-quick-wrap")) setQuick(null);
    };
    document.addEventListener("mousedown", onDown, true);
    return () => document.removeEventListener("mousedown", onDown, true);
  }, [quick]);

  const goLocal = useCallback(
    async (path: string) => {
      setRenaming(null);
      const my = ++lseq.current;
      try {
        const r = await invoke<{ parent: string; entries: Entry[] }>("local_list", { path });
        if (my !== lseq.current) return;
        setLocal({ path, parent: r.parent, entries: r.entries, sel: null });
        setError(null);
      } catch (e) {
        if (my === lseq.current) showErr(e);
      }
    },
    [showErr],
  );

  const goRemote = useCallback(
    async (path: string) => {
      if (!tabId) return;
      setRenaming(null);
      const my = ++rseq.current;
      try {
        const entries = await invoke<Entry[]>("sftp_list", { id: tabId, path });
        if (my !== rseq.current) return;
        setRemote({ path, parent: remoteParent(path), entries, sel: null });
        setError(null);
      } catch (e) {
        if (my === rseq.current) showErr(e);
      }
    },
    [tabId, showErr],
  );

  useEffect(() => {
    invoke<string>("local_home")
      .then(goLocal)
      .catch(showErr);
  }, [goLocal, showErr]);

  /// 换标签页 = 换一条独立的 SFTP 连接；面板关掉时连接跟着关
  useEffect(() => {
    if (!tabId || !params) {
      setRemoteState("idle");
      setRemote(emptySide);
      return;
    }
    let gone = false;
    setRemoteState("connecting");
    setRemote(emptySide);
    setError(null);
    invoke<string>("sftp_open", {
      args: {
        id: tabId,
        host: params.host,
        port: params.port,
        username: params.username,
        password: params.password,
        keyPath: params.keyPath,
      },
    })
      .then((home) => {
        if (gone) return;
        setRemoteState("ready");
        return goRemote(home);
      })
      .catch((e) => {
        if (!gone) setError(`SFTP 连接失败：${e}`);
      });
    const id = tabId;
    return () => {
      gone = true;
      invoke("sftp_close", { id }).catch(() => {});
    };
    // goRemote 只随 tabId 变，列在这里会让每次切换多跑一次 close+open
  }, [tabId, params]);

  const transfer = async (run: (onEvent: Channel<ProgEvent>) => Promise<void>, then: () => Promise<void>) => {
    setProg({ name: "", done: 0, total: 0 });
    setError(null);
    try {
      const ch = new Channel<ProgEvent>();
      ch.onmessage = (ev) => {
        if (ev.event === "tick") setProg({ name: ev.name, done: ev.done, total: ev.total });
      };
      await run(ch);
      await then();
    } catch (e) {
      setError(String(e));
    } finally {
      setProg(null);
    }
  };

  const upload = () => {
    if (!tabId || !local.sel || local.sel.dir || !remote.path) return;
    const file = local.sel.path;
    void transfer(
      (onEvent) => invoke("sftp_upload", { id: tabId, local: file, remoteDir: remote.path, onEvent }),
      () => goRemote(remote.path),
    );
  };

  const download = () => {
    if (!tabId || !remote.sel || !local.path) return;
    const file = remote.sel.path;
    void transfer(
      (onEvent) => invoke("sftp_download", { id: tabId, remote: file, localDir: local.path, onEvent }),
      () => goLocal(local.path),
    );
  };

  const target = focus === "local" ? local : remote;

  const rename = () => {
    const t = target.sel;
    if (t) setRenaming(t.path);
  };

  const newDir = () => {
    if (!target.path) return;
    setRenaming(NEW_ROW);
  };

  const cancelEdit = () => setRenaming(null);

  /// 提交行内编辑：哨兵 NEW_ROW 是新建目录，否则按条目路径改名
  const commitEdit = (side: "local" | "remote", entry: Entry | null, raw: string) => {
    setRenaming(null);
    const name = raw.trim();
    if (!name) return;
    const back = () => (side === "local" ? goLocal(local.path) : goRemote(remote.path));
    let job: Promise<unknown>;
    if (!entry) {
      job =
        side === "local"
          ? invoke("local_mkdir", { dir: local.path, name })
          : invoke("sftp_mkdir", { id: tabId, path: remoteChild(remote.path, name) });
    } else {
      if (name === entry.name) return;
      job =
        side === "local"
          ? invoke("local_rename", { path: entry.path, name })
          : invoke("sftp_rename", { id: tabId, old: entry.path, new: remoteChild(remote.path, name) });
    }
    job.then(back).catch(showErr);
  };

  const del = () => {
    const t = target.sel;
    if (!t || !window.confirm(`确定删除 "${t.name}" 吗？`)) return;
    const back = () => (focus === "local" ? goLocal(local.path) : goRemote(remote.path));
    const job =
      focus === "local"
        ? invoke("local_remove", { path: t.path, dir: t.dir })
        : invoke("sftp_remove", { id: tabId, path: t.path, dir: t.dir });
    job.then(back).catch(showErr);
  };

  const renderPane = (
    side: "local" | "remote",
    title: string,
    data: Side,
    setData: (s: Side) => void,
    go: (p: string) => Promise<void>,
    home: React.ReactNode,
    hint: string,
  ) => (
    <div
      className={`fp-pane${focus === side ? " focus" : ""}`}
      onMouseDown={() => {
        if (focus !== side) setRenaming(null);
        setFocus(side);
      }}
    >
      <div className="fp-head">
        <span className="fp-title" title={title}>
          {title}
        </span>
        <span className="fp-head-actions">
          <button
            title="上一级"
            disabled={!data.path}
            onClick={() => void go(side === "local" ? data.parent : remoteParent(data.path))}
          >
            <IconArrowBarUp {...ink(15)} />
          </button>
          <button title="刷新" disabled={!data.path} onClick={() => void go(data.path)}>
            <IconRefresh {...ink(15)} />
          </button>
          {home}
        </span>
      </div>
      <input
        className="fp-path"
        value={data.path}
        placeholder="路径，回车跳转"
        spellCheck={false}
        onChange={(e) => setData({ ...data, path: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter") void go(e.currentTarget.value.trim());
        }}
      />
      <div className="fp-crumbs">
        {crumbs(data.path).map((c) => (
          <span key={c.path} className="fp-crumb" onClick={() => void go(c.path)}>
            {c.label}
          </span>
        ))}
      </div>
      <div className="fp-list">
        {renaming === NEW_ROW && focus === side && (
          <div className="fp-row selected">
            <span className="tree-icon">
              <IconFolder {...ink(15)} />
            </span>
            <InlineEdit initial="" onCommit={(name) => commitEdit(side, null, name)} onCancel={cancelEdit} />
          </div>
        )}
        {data.entries.map((e) =>
          renaming === e.path && focus === side ? (
            <div key={e.path} className="fp-row selected" title={e.path}>
              <span className="tree-icon">{e.dir ? <IconFolder {...ink(15)} /> : <IconFile {...ink(15)} />}</span>
              <InlineEdit initial={e.name} onCommit={(name) => commitEdit(side, e, name)} onCancel={cancelEdit} />
            </div>
          ) : (
            <div
              key={e.path}
              className={`fp-row${data.sel?.path === e.path ? " selected" : ""}`}
              title={e.path}
              onClick={() => setData({ ...data, sel: e })}
              onDoubleClick={() => {
                if (e.dir) void go(e.path);
                else if (side === "remote") download();
              }}
            >
              <span className="tree-icon">{e.dir ? <IconFolder {...ink(15)} /> : <IconFile {...ink(15)} />}</span>
              <span className="fp-name">{e.name}</span>
              <span className="fp-size">{e.dir ? "" : fmtSize(e.size)}</span>
              <span className="fp-date">{fmtDate(e.mtime)}</span>
            </div>
          ),
        )}
        {data.entries.length === 0 && renaming !== NEW_ROW && <p className="fp-empty">{hint || "空目录"}</p>}
      </div>
    </div>
  );

  const pct = prog && prog.total ? Math.min(100, Math.round((prog.done / prog.total) * 100)) : 0;
  const remoteHint =
    remoteState === "connecting" ? "连接中…" : remoteState === "idle" ? (tabId ? "未连接" : "没有活动标签页") : "";

  return (
    <div className="filepanel">
      <div className="fp-bar">
        <button
          title="上传：本地选中项 → 远端当前目录"
          disabled={!local.sel || local.sel.dir || remoteState !== "ready" || !!prog}
          onClick={upload}
        >
          <IconFileUpload {...ink(16)} />
        </button>
        <button
          title="下载：远端选中项 → 本地当前目录"
          disabled={!remote.sel || remote.sel.dir || !local.path || !!prog}
          onClick={download}
        >
          <IconFileDownload {...ink(16)} />
        </button>
        <span className="tb-sep" />
        <button
          title={`在${focus === "local" ? "本地" : "远端"}当前目录新建子目录`}
          disabled={!target.path}
          onClick={newDir}
        >
          <IconFolderPlus {...ink(16)} />
        </button>
        <button title="重命名选中项" disabled={!target.sel} onClick={rename}>
          <IconPencil {...ink(16)} />
        </button>
        <button title="删除选中项" disabled={!target.sel} onClick={del}>
          <IconTrash {...ink(16)} />
        </button>
        <span className="fp-spacer" />
        <button title="关闭文件面板" onClick={onClose}>
          <IconX {...ink(16)} />
        </button>
      </div>

      <div className="fp-body">
        {renderPane(
          "local",
          "本地",
          local,
          setLocal,
          goLocal,
          <>
            <button title="主目录" onClick={() => void invoke<string>("local_home").then(goLocal).catch(showErr)}>
              <IconHome {...ink(15)} />
            </button>
            <span className="fp-quick-wrap">
              <button
                title="快速访问：常用文件夹和驱动器"
                aria-expanded={!!quick}
                aria-haspopup="true"
                onClick={toggleQuick}
              >
                <IconFolderStar {...ink(15)} />
              </button>
              {quick && (
                <div className="fp-quick">
                  {quick.map((s) => (
                    <div
                      key={s.path || s.name}
                      className="fp-quick-row"
                      title={s.path}
                      onClick={() => {
                        setQuick(null);
                        void goLocal(s.path);
                      }}
                    >
                      {s.name}
                    </div>
                  ))}
                </div>
              )}
            </span>
          </>,
          "",
        )}
        {renderPane(
          "remote",
          params ? `${params.username}@${params.host}` : "远端",
          remote,
          setRemote,
          goRemote,
          <button
            title="服务器根目录"
            disabled={remoteState !== "ready"}
            onClick={() => void goRemote("/")}
          >
            <IconServer {...ink(15)} />
          </button>,
          remoteHint,
        )}
      </div>

      <div className="fp-foot">
        {prog ? (
          <>
            <span className="fp-prog-name">{prog.name}</span>
            <span className="fp-prog-track">
              <i style={{ width: `${pct}%` }} />
            </span>
            <span className="fp-prog-pct">{pct}%</span>
          </>
        ) : (
          <span className="fp-status">
            {target.sel
              ? `已选中 ${target.sel.name} · ${focus === "local" ? "本地" : "远端"}`
              : `${params ? `${params.username}@${params.host}` : "未连接"} · 点选一侧后使用新建/重命名/删除`}
          </span>
        )}
      </div>
      {error && <div className="fp-error">{error}</div>}
    </div>
  );
}
