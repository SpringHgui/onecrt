import { useState } from "react";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";

interface Props {
  host: string;
  username: string;
  onClose(): void;
  onSkip(user: string): void;
  onSubmit(user: string, password: string, save: boolean): void;
}

export default function PasswordPrompt({ host, username, onClose, onSkip, onSubmit }: Props) {
  const [user, setUser] = useState(username);
  const [password, setPassword] = useState("");
  const [save, setSave] = useState(false);

  const ok = () => {
    const u = user.trim();
    if (!u || !password) return;
    onSubmit(u, password, save);
  };

  return (
    <div className="dialog-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className="dialog"
        onKeyDown={(e) => {
          if (e.key === "Enter") ok();
        }}
      >
        <div className="dialog-titlebar">
          <span>Enter Secure Shell Password</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <p className="pwd-note">
            {user.trim() || "?"}@{host} requires a password.
            <br />
            <span>Please enter a password now.</span>
          </p>
          <label className="form-row">
            <span>Username:</span>
            <input value={user} onChange={(e) => setUser(e.target.value)} />
          </label>
          <label className="form-row">
            <span>Password:</span>
            <input type="password" value={password} autoFocus onChange={(e) => setPassword(e.target.value)} />
          </label>
          <div className="check-rows">
            <label>
              <input type="checkbox" checked={save} onChange={(e) => setSave(e.target.checked)} />
              Save password
            </label>
          </div>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" disabled={!user.trim() || !password} onClick={ok}>
            OK
          </button>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn" onClick={() => onSkip(user.trim())}>
            Skip
          </button>
        </div>
      </div>
    </div>
  );
}
