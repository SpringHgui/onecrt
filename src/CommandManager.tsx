import {
  IconChevronDown,
  IconChevronRight,
  IconClipboard,
  IconCopy,
  IconFolder,
  IconFolderOpen,
  IconFolderPlus,
  IconPencil,
  IconScissors,
  IconSettings,
  IconTrash,
} from "@tabler/icons-react";
import { ink } from "./icons";
import InlineEdit from "./InlineEdit";
import { CommandNode, SavedCommand, isCommandFolder } from "./types";

interface Props {
  nodes: CommandNode[];
  expanded: Set<string>;
  selectedId: string | null;
  /// 处于行内改名状态的节点 id，null 表示没有
  renamingId: string | null;
  hasClipboard: boolean;
  canSend: boolean;
  filter: string;
  onFilter(v: string): void;
  onToggle(id: string): void;
  onSelect(id: string): void;
  /** 工具栏动作都作用于当前选中项，未选中时按钮为禁用态 */
  onSend(cmd: SavedCommand): void;
  onNewCommand(): void;
  onNewFolder(): void;
  onCut(): void;
  onCopy(): void;
  onPaste(): void;
  onDelete(): void;
  onProperties(): void;
  onContextMenu(node: CommandNode, e: React.MouseEvent): void;
  onRenameCommit(node: CommandNode, name: string): void;
  onRenameCancel(): void;
}

function render(nodes: CommandNode[], depth: number, props: Props): React.ReactNode {
  return nodes.map((n) => {
    const folder = isCommandFolder(n);
    const open = folder && props.expanded.has(n.id);
    const command = n.kind === "command" ? n : null;
    const editing = props.renamingId === n.id;
    const row = (
      <div
        key={n.id}
        className={`tree-row${props.selectedId === n.id ? " selected" : ""}`}
        style={{ paddingLeft: 6 + depth * 16 }}
        onClick={() => props.onSelect(n.id)}
        onDoubleClick={() => {
          if (!editing && command && props.canSend) props.onSend(command);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          props.onSelect(n.id);
          props.onContextMenu(n, e);
        }}
      >
        {folder ? (
          <span
            className="tree-caret"
            onClick={(e) => {
              e.stopPropagation();
              props.onToggle(n.id);
            }}
          >
            {open ? <IconChevronDown {...ink(14)} /> : <IconChevronRight {...ink(14)} />}
          </span>
        ) : (
          <span className="tree-caret placeholder" />
        )}
        <span className="tree-icon">
          {folder ? (
            open ? <IconFolderOpen {...ink(15)} /> : <IconFolder {...ink(15)} />
          ) : (
            <span className="cmd-dot" title={command!.text} />
          )}
        </span>
        {editing ? (
          <InlineEdit
            initial={n.name}
            onCommit={(name) => props.onRenameCommit(n, name)}
            onCancel={props.onRenameCancel}
          />
        ) : (
          <span className="tree-label">{n.name}</span>
        )}
      </div>
    );
    if (!folder || !open) return row;
    return (
      <div key={n.id}>
        {row}
        {render(n.children, depth + 1, props)}
      </div>
    );
  });
}

export default function CommandManager(props: Props) {
  const selected: CommandNode | null = props.selectedId
    ? (function find(ns: CommandNode[]): CommandNode | null {
        for (const n of ns) {
          if (n.id === props.selectedId) return n;
          if (isCommandFolder(n)) {
            const hit = find(n.children);
            if (hit) return hit;
          }
        }
        return null;
      })(props.nodes)
    : null;
  const selectedCmd = selected?.kind === "command" ? selected : null;

  return (
    <div className="cmd-manager">
      <div className="sidebar-header">
        <span>Command Manager</span>
      </div>
      <div className="sidebar-toolbar">
        <button
          title="发送命令"
          disabled={!selectedCmd || !props.canSend}
          onClick={() => selectedCmd && props.onSend(selectedCmd)}
        >
          <IconChevronRight {...ink(16)} />
        </button>
        <button title="新建命令" onClick={props.onNewCommand}>
          <IconPencil {...ink(16)} />
        </button>
        <button title="新建文件夹" onClick={props.onNewFolder}>
          <IconFolderPlus {...ink(16)} />
        </button>
        <button title="剪切" disabled={!selected} onClick={props.onCut}>
          <IconScissors {...ink(16)} />
        </button>
        <button title="复制" disabled={!selected} onClick={props.onCopy}>
          <IconCopy {...ink(16)} />
        </button>
        <button title="粘贴" disabled={!props.hasClipboard} onClick={props.onPaste}>
          <IconClipboard {...ink(16)} />
        </button>
        <button title="删除" disabled={!selected} onClick={props.onDelete}>
          <IconTrash {...ink(16)} />
        </button>
        <button title="属性" disabled={!selected} onClick={props.onProperties}>
          <IconSettings {...ink(16)} />
        </button>
      </div>
      <input
        className="filter-input"
        placeholder="Filter by folder/command name"
        value={props.filter}
        onChange={(e) => props.onFilter(e.target.value)}
      />
      <div className="sidebar-tree">
        <div className="session-tree">{render(props.nodes, 0, props)}</div>
      </div>
    </div>
  );
}
