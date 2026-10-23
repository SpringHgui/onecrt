import { IconChevronDown, IconChevronRight, IconDeviceDesktop, IconFolder, IconFolderOpen } from "@tabler/icons-react";
import { ink } from "./icons";
import InlineEdit from "./InlineEdit";
import { Node, isFolder } from "./types";

interface Props {
  nodes: Node[];
  expanded: Set<string>;
  selectedId: string | null;
  /// 处于行内改名状态的节点 id，null 表示没有
  renamingId: string | null;
  onToggle(id: string): void;
  onSelect(id: string): void;
  onActivate(node: Node): void;
  onDelete(): void;
  onContextMenu(node: Node, e: React.MouseEvent): void;
  onRenameCommit(node: Node, name: string): void;
  onRenameCancel(): void;
}

function render(
  nodes: Node[],
  depth: number,
  props: Props,
  keyPrefix: string,
): React.ReactNode {
  return nodes.map((n) => {
    const active = props.selectedId === n.id;
    const editing = props.renamingId === n.id;
    const row = (
      <div
        key={n.id}
        className={`tree-row${active ? " selected" : ""}`}
        style={{ paddingLeft: 6 + depth * 16 }}
        onClick={() => props.onSelect(n.id)}
        onDoubleClick={() => {
          if (!editing) props.onActivate(n);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          props.onSelect(n.id);
          props.onContextMenu(n, e);
        }}
      >
        {isFolder(n) ? (
          <span
            className="tree-caret"
            onClick={(e) => {
              e.stopPropagation();
              props.onToggle(n.id);
            }}
          >
            {props.expanded.has(n.id) ? <IconChevronDown {...ink(14)} /> : <IconChevronRight {...ink(14)} />}
          </span>
        ) : (
          <span className="tree-caret placeholder" />
        )}
        <span className="tree-icon">
          {isFolder(n) ? (
            props.expanded.has(n.id) ? <IconFolderOpen {...ink(15)} /> : <IconFolder {...ink(15)} />
          ) : (
            <IconDeviceDesktop {...ink(15)} />
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
    if (isFolder(n) && props.expanded.has(n.id)) {
      return (
        <div key={n.id}>
          {row}
          {render(n.children, depth + 1, props, `${keyPrefix}${n.id}/`)}
        </div>
      );
    }
    return row;
  });
}

export default function SessionTree(props: Props) {
  return (
    // tabIndex 让整棵树能接住焦点：点过行之后 Del 才知该作用于哪个面板
    <div
      className="session-tree"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key !== "Delete" || (e.target as HTMLElement).tagName === "INPUT") return;
        e.preventDefault();
        props.onDelete();
      }}
    >
      {render(props.nodes, 0, props, "")}
    </div>
  );
}
