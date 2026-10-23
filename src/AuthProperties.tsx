import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconX } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";

export interface AuthPropsResult {
  password: string;
  storePassword: boolean;
  keyPath: string;
}

interface Props {
  method: string;
  password: string;
  storePassword: boolean;
  keyPath: string;
  onClose(): void;
  onOk(r: AuthPropsResult): void;
}

const TITLE_KEY: Record<string, string> = {
  PublicKey: "ap.publicKeyTitle",
  Password: "ap.passwordTitle",
  "Keyboard Interactive": "ap.kbdTitle",
};

export default function AuthPropertiesDialog({ method, password, storePassword, keyPath, onClose, onOk }: Props) {
  const { t } = useI18n();
  const isKey = method === "PublicKey";
  const [store, setStore] = useState(storePassword);
  const [pwd, setPwd] = useState(password);
  const [confirm, setConfirm] = useState(password);
  const [key, setKey] = useState(keyPath);
  const [fp, setFp] = useState("");

  useEffect(() => {
    if (!isKey) return;
    const t = setTimeout(() => {
      if (!key.trim()) {
        setFp("");
        return;
      }
      invoke<string>("key_fingerprint", { path: key })
        .then(setFp)
        .catch((e) => setFp(String(e)));
    }, 250);
    return () => clearTimeout(t);
  }, [key, isKey]);

  const mismatch = store && pwd !== confirm;

  const browse = async () => {
    const picked = await invoke<string | null>("pick_key_file").catch(() => null);
    if (picked) setKey(picked);
  };

  return (
    <div className="dialog-overlay props-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog props-dialog">
        <div className="dialog-titlebar">
          <span>{TITLE_KEY[method] ? t(TITLE_KEY[method]) : t("ap.genericTitle", { method })}</span>
          <button className="dialog-close" onClick={onClose}>
            <IconX {...ink(16)} />
          </button>
        </div>
        <div className="dialog-body">
          {isKey ? (
            <fieldset className="props-group">
              <legend>{t("ap.identity")}</legend>
              <div className="form-row">
                <span>{t("ap.identityFile")}</span>
                <input
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                  placeholder={t("qc.keyPlaceholder")}
                />
                <button className="btn btn-narrow" title={t("ap.browse")} onClick={browse}>
                  ...
                </button>
              </div>
              <div className="form-row fp-row">
                <span>{t("ap.fingerprint")}</span>
                <textarea readOnly value={fp} placeholder={t("ap.fpPlaceholder")} />
              </div>
            </fieldset>
          ) : (
            <fieldset className="props-group">
              <legend>{t("ap.password")}</legend>
              <label className="check-line">
                <input type="checkbox" checked={store} onChange={(e) => setStore(e.target.checked)} />
                {t("ap.storePassword")}
              </label>
              <input type="password" value={pwd} disabled={!store} onChange={(e) => setPwd(e.target.value)} />
              <div className="props-label">{t("ap.confirmPassword")}</div>
              <input type="password" value={confirm} disabled={!store} onChange={(e) => setConfirm(e.target.value)} />
              {mismatch && <div className="hint-err">{t("ap.mismatch")}</div>}
              <div className="props-note">{t("ap.encryptedNote")}</div>
            </fieldset>
          )}
        </div>
        <div className="dialog-footer">
          <button
            className="btn btn-primary"
            disabled={mismatch}
            onClick={() =>
              onOk({ password: store ? pwd : "", storePassword: store, keyPath: isKey ? key.trim() : keyPath })
            }
          >
            {t("ap.ok")}
          </button>
          <button className="btn" onClick={onClose}>
            {t("ap.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
