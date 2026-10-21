import { useState } from "react";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";

export interface CommandFields {
  name: string;
  /** 发往终端的文本，不含结尾换行 */
  text: string;
}

interface Props {
  title: string;
  initial?: Partial<CommandFields>;
  /** 文件夹只有 Name 可改，命令才有 Command 文本 */
  showText?: boolean;
  onClose(): void;
  onSubmit(f: CommandFields): void;
}

export default function CommandProperties({ title, initial, showText = true, onClose, onSubmit }: Props) {
  const [name, setName] = useState(initial?.name ?? "");
  const [text, setText] = useState(initial?.text ?? "");

  const submit = () => {
    const n = name.trim();
    if (!n) return;
    onSubmit({ name: n, text: showText ? text.trim() || n : n });
  };

  return (
    <div className="dialog-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className="dialog"
        onKeyDown={(e) => {
          // 命令文本框里回车是换行，不能当确认
          if (e.key === "Enter" && (e.target as HTMLElement).tagName !== "TEXTAREA") submit();
        }}
      >
        <div className="dialog-titlebar">
          <span>{title}</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <label className="form-row">
            <span>Name:</span>
            <input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          {showText && (
            <label className="form-row form-col">
              <span>Command:</span>
              <textarea
                className="cmd-textarea"
                rows={4}
                spellCheck={false}
                value={text}
                placeholder="留空则使用名称作为命令文本"
                onChange={(e) => setText(e.target.value)}
              />
            </label>
          )}
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" disabled={!name.trim()} onClick={submit}>
            OK
          </button>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
