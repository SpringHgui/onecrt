import { useState } from "react";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";

interface Props {
  host: string;
  username: string;
  onClose(): void;
  onSkip(user: string): void;
  onSubmit(user: string, password: string, save: boolean): void;
}

export default function PasswordPrompt({ host, username, onClose, onSkip, onSubmit }: Props) {
  const { t } = useI18n();
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
          <span>{t("pwd.title")}</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          <p className="pwd-note">
            {t("pwd.requires", { user: user.trim() || "?", host })}
            <br />
            <span>{t("pwd.enterNow")}</span>
          </p>
          <label className="form-row">
            <span>{t("pwd.username")}</span>
            <input value={user} onChange={(e) => setUser(e.target.value)} />
          </label>
          <label className="form-row">
            <span>{t("pwd.password")}</span>
            <input type="password" value={password} autoFocus onChange={(e) => setPassword(e.target.value)} />
          </label>
          <div className="check-rows">
            <label>
              <input type="checkbox" checked={save} onChange={(e) => setSave(e.target.checked)} />
              {t("pwd.save")}
            </label>
          </div>
        </div>
        <div className="dialog-footer">
          <button className="btn btn-primary" disabled={!user.trim() || !password} onClick={ok}>
            {t("pwd.ok")}
          </button>
          <button className="btn" onClick={onClose}>
            {t("pwd.cancel")}
          </button>
          <button className="btn" onClick={() => onSkip(user.trim())}>
            {t("pwd.skip")}
          </button>
        </div>
      </div>
    </div>
  );
}
