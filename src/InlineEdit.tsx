import { useEffect, useRef } from "react";

interface Props {
  initial: string;
  onCommit(name: string): void;
  onCancel(): void;
  className?: string;
}

/// 行内改名：聚焦全选，Enter 提交 / Esc 取消，失焦等同取消
const log = (s: string) => {
  const w = window as unknown as { __ieLog?: string[] };
  (w.__ieLog ??= []).push(s);
};

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
        log(`key:${e.key}`);
        if (e.key === "Enter") {
          done.current = true;
          log("commit");
          commit(e.currentTarget.value.trim());
        } else if (e.key === "Escape") {
          done.current = true;
          log("cancel");
          cancel();
        }
      }}
      onBlur={() => {
        log(`blur done=${done.current}`);
        if (!done.current) cancel();
      }}
    />
  );
}
