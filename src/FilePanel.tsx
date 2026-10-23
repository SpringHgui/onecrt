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
import { fmtSize } from "./format";
import { ink } from "./icons";
import ConfirmDialog from "./ConfirmDialog";
import InlineEdit from "./InlineEdit";
import PanelPin from "./PanelPin";
import { ConnectParams } from "./QuickConnect";
import { enabledMethods } from "./types";
import { beginTransfer, endTransfer, reportTransfer, useTransferActivity, type TransferDir } from "./transfers";
import { useI18n } from "./i18n";
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

type ProgEvent =
  | { event: "tick"; name: string; done: number; total: number }
  | { event: "done"; name: string };

type RemoteState = "idle" | "connecting" | "ready";

const emptySide: Side = { path: "", parent: "", entries: [], sel: null };

/// renaming 的哨兵值：表示当前是"新建目录"那一行，而不是某个已存在条目在改名
const NEW_ROW = "\0new";

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
  /// 传输开始时通知 App 放出底部的传输面板
  onTransferStart?(): void;
  /// 顶栏右上角的图钉：钉住=常驻，取消=自动隐藏
  pinned?: boolean;
  onTogglePin?(): void;
  /// 顶栏作为拖拽把手（flexlayout 拖放），由 App 注入
  drag?: React.HTMLAttributes<HTMLDivElement>;
}

export default function FilePanel({ tabId, params, onClose, onTransferStart, pinned, onTogglePin, drag }: Props) {
  const { t } = useI18n();
  const [local, setLocal] = useState<Side>(emptySide);
  const [remote, setRemote] = useState<Side>(emptySide);
  const [remoteState, setRemoteState] = useState<RemoteState>("idle");
  /// 新建/重命名/删除作用于哪一侧；点任一栏即切换
  const [focus, setFocus] = useState<"local" | "remote">("remote");
  /// 进度记录在全局 store 里（底部传输面板和状态栏读），这里只需要"有没有在传"
  const busy = useTransferActivity();
  const [error, setError] = useState<string | null>(null);
  /// 正在行内改名的条目路径，null 表示没有
  const [renaming, setRenaming] = useState<string | null>(null);
  /// 删除确认框：记下条目和所在侧，点确定时才真删
  const [confirmDel, setConfirmDel] = useState<{ entry: Entry; side: "local" | "remote" } | null>(null);
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
        methods: enabledMethods(params.auth),
      },
    })
      .then((home) => {
        if (gone) return;
        setRemoteState("ready");
        return goRemote(home);
      })
      .catch((e) => {
        if (!gone) setError(t("fp.sftpError", { e: String(e) }));
      });
    const id = tabId;
    return () => {
      gone = true;
      invoke("sftp_close", { id }).catch(() => {});
    };
    // goRemote 只随 tabId 变，列在这里会让每次切换多跑一次 close+open
  }, [tabId, params]);

  /// 一次传输：进度写进全局 store，传完再刷新对应一侧的列表
  const transfer = async (
    dir: TransferDir,
    file: Entry,
    run: (onEvent: Channel<ProgEvent>) => Promise<void>,
    then: () => Promise<void>,
  ) => {
    const id = beginTransfer(dir, file.name, file.size ?? 0);
    onTransferStart?.();
    setError(null);
    try {
      const ch = new Channel<ProgEvent>();
      ch.onmessage = (ev) => {
        if (ev.event === "tick") reportTransfer(id, ev.done, ev.total);
      };
      await run(ch);
      endTransfer(id);
      await then();
    } catch (e) {
      const msg = String(e);
      endTransfer(id, msg);
      setError(msg);
    }
  };

  const upload = () => {
    if (!tabId || !local.sel || local.sel.dir || !remote.path || busy) return;
    const file = local.sel;
    void transfer(
      "up",
      file,
      (onEvent) => invoke("sftp_upload", { id: tabId, local: file.path, remoteDir: remote.path, onEvent }),
      () => goRemote(remote.path),
    );
  };

  const download = () => {
    if (!tabId || !remote.sel || !local.path || busy) return;
    const file = remote.sel;
    void transfer(
      "down",
      file,
      (onEvent) => invoke("sftp_download", { id: tabId, remote: file.path, localDir: local.path, onEvent }),
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
    if (!t) return;
    setConfirmDel({ entry: t, side: focus });
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
            title={t("fp.up")}
            disabled={!data.path}
            onClick={() => void go(side === "local" ? data.parent : remoteParent(data.path))}
          >
            <IconArrowBarUp {...ink(15)} />
          </button>
          <button title={t("fp.refresh")} disabled={!data.path} onClick={() => void go(data.path)}>
            <IconRefresh {...ink(15)} />
          </button>
          {home}
        </span>
      </div>
      <input
        className="fp-path"
        value={data.path}
        placeholder={t("fp.pathPlaceholder")}
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
        {data.entries.length === 0 && renaming !== NEW_ROW && <p className="fp-empty">{hint || t("fp.empty")}</p>}
      </div>
    </div>
  );

  const remoteHint =
    remoteState === "connecting"
      ? t("fp.connecting")
      : remoteState === "idle"
        ? tabId
          ? t("fp.notConnected")
          : t("fp.noActiveTab")
        : "";

  return (
    <div className="filepanel">
      <div className="fp-bar" {...drag}>
        <button
          title={t("fp.upload")}
          disabled={!local.sel || local.sel.dir || remoteState !== "ready" || busy}
          onClick={upload}
        >
          <IconFileUpload {...ink(16)} />
        </button>
        <button
          title={t("fp.download")}
          disabled={!remote.sel || remote.sel.dir || !local.path || busy}
          onClick={download}
        >
          <IconFileDownload {...ink(16)} />
        </button>
        <span className="tb-sep" />
        <button
          title={focus === "local" ? t("fp.newDirLocal") : t("fp.newDirRemote")}
          disabled={!target.path}
          onClick={newDir}
        >
          <IconFolderPlus {...ink(16)} />
        </button>
        <button title={t("fp.rename")} disabled={!target.sel} onClick={rename}>
          <IconPencil {...ink(16)} />
        </button>
        <button title={t("fp.delete")} disabled={!target.sel} onClick={del}>
          <IconTrash {...ink(16)} />
        </button>
        <span className="fp-spacer" />
        {onTogglePin && <PanelPin pinned={!!pinned} onToggle={onTogglePin} />}
        <button title={t("fp.close")} onClick={onClose}>
          <IconX {...ink(16)} />
        </button>
      </div>

      <div className="fp-body">
        {renderPane(
          "local",
          t("fp.local"),
          local,
          setLocal,
          goLocal,
          <>
            <button title={t("fp.home")} onClick={() => void invoke<string>("local_home").then(goLocal).catch(showErr)}>
              <IconHome {...ink(15)} />
            </button>
            <span className="fp-quick-wrap">
              <button
                title={t("fp.quickAccess")}
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
          params ? `${params.username}@${params.host}` : t("fp.remote"),
          remote,
          setRemote,
          goRemote,
          <button
            title={t("fp.serverRoot")}
            disabled={remoteState !== "ready"}
            onClick={() => void goRemote("/")}
          >
            <IconServer {...ink(15)} />
          </button>,
          remoteHint,
        )}
      </div>

      {/* 进度条不画在这里：传输过程统一显示在底部的传输面板和状态栏 */}
      <div className="fp-foot">
        <span className="fp-status">
          {target.sel
            ? t("fp.selected", { name: target.sel.name, side: focus === "local" ? t("fp.local") : t("fp.remote") })
            : t("fp.footHint", { conn: params ? `${params.username}@${params.host}` : t("fp.notConnected") })}
        </span>
      </div>
      {error && <div className="fp-error">{error}</div>}
      {confirmDel && (
        <ConfirmDialog
          message={t("common.deleteConfirm", { name: confirmDel.entry.name })}
          onCancel={() => setConfirmDel(null)}
          onConfirm={() => {
            const { entry, side } = confirmDel;
            setConfirmDel(null);
            const back = () => (side === "local" ? goLocal(local.path) : goRemote(remote.path));
            const job =
              side === "local"
                ? invoke("local_remove", { path: entry.path, dir: entry.dir })
                : invoke("sftp_remove", { id: tabId, path: entry.path, dir: entry.dir });
            job.then(back).catch(showErr);
          }}
        />
      )}
    </div>
  );
}
