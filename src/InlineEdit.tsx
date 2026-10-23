import { useEffect, useRef } from "react";

interface Props {
  initial: string;
  onCommit(name: string): void;
  onCancel(): void;
  className?: string;
}

/// 行内改名：聚焦全选，Enter 提交 / Esc 取消，失焦等同取消
export default function InlineEdit({ initial, onCommit, onCancel, className }: Props) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  const commit = onCommit;
  const cancel = onCancel;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    const i = el.value.lastIndexOf(".");
    el.setSelectionRange(i > 0 ? i : 0, el.value.length);
    // 树是可滚动的：占位行落在折叠区外时用户看不见，会以为"点了没反应"
    el.scrollIntoView({ block: "nearest" });
  }, []);

  return (
    <input
      ref={ref}
      className={`inline-edit${className ? " " + className : ""}`}
      defaultValue={initial}
      spellCheck={false}
      onMouseDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          done.current = true;
          commit(e.currentTarget.value.trim());
        } else if (e.key === "Escape") {
          // flexlayout 把 Escape 绑成"收起 overlay 面板"（document 上的原生监听）。
          // 不拦下来，取消改名的同时整个面板会被关掉
          e.preventDefault();
          e.stopPropagation();
          done.current = true;
          cancel();
        }
      }}
      onBlur={() => {
        if (!done.current) cancel();
      }}
    />
  );
}
