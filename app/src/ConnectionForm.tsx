import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type SavedConnection, type SslMode, type Target } from "./api";

const NEW_POSTGRES: SavedConnection = {
  id: "",
  name: "",
  target: { engine: "postgres", host: "localhost", port: 5432, user: "postgres", password: null, database: "postgres", ssl: "prefer" },
  agent: false,
};

type Props = {
  initial: SavedConnection | null;
  onSaved: (c: SavedConnection) => void;
  onDeleted: (id: string) => void;
  onClose: () => void;
};

export function ConnectionForm({ initial, onSaved, onDeleted, onClose }: Props) {
  const [conn, setConn] = useState<SavedConnection>(initial ?? NEW_POSTGRES);
  // null = leave the saved password alone
  const [password, setPassword] = useState<string | null>(null);
  const [hasSaved, setHasSaved] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (initial?.id) api.hasPassword(initial.id).then(setHasSaved, () => {});
  }, [initial?.id]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const t = conn.target;
  const setTarget = (patch: Partial<Target>) => setConn({ ...conn, target: { ...t, ...patch } as Target });

  async function pasteUrl(url: string) {
    try {
      const parsed = await api.parseUrl(url.trim());
      setConn({ ...conn, name: conn.name || parsed.database, target: { engine: "postgres", ...parsed, password: null } });
      if (parsed.password) setPassword(parsed.password);
      setStatus(null);
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  }

  async function test() {
    setBusy(true);
    setStatus(null);
    try {
      setStatus({ ok: true, text: await api.testConnection(conn, password) });
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    const name = conn.name.trim() || (t.engine === "postgres" ? `${t.database}@${t.host}` : t.path.split("/").pop()!);
    try {
      onSaved(await api.saveConnection({ ...conn, name }, password));
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  }

  async function remove() {
    if (!initial?.id || !confirm(`Delete “${initial.name}”? The saved password is removed too.`)) return;
    await api.deleteConnection(initial.id);
    onDeleted(initial.id);
  }

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <form
        className="modal"
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <h2>{initial?.id ? "Edit connection" : "New connection"}</h2>

        <label>
          Name
          <input autoFocus value={conn.name} onChange={(e) => setConn({ ...conn, name: e.target.value })} placeholder="optional" />
        </label>

        {t.engine === "postgres" ? (
          <>
            <label>
              URL
              <input
                spellCheck={false}
                placeholder="paste postgres://… to fill the fields"
                onPaste={(e) => {
                  e.preventDefault();
                  pasteUrl(e.clipboardData.getData("text"));
                }}
              />
            </label>
            <div className="row">
              <label className="grow">
                Host
                <input spellCheck={false} value={t.host} onChange={(e) => setTarget({ host: e.target.value })} />
              </label>
              <label className="port">
                Port
                <input
                  inputMode="numeric"
                  value={t.port}
                  onChange={(e) => setTarget({ port: Number(e.target.value.replace(/\D/g, "")) || 0 })}
                />
              </label>
            </div>
            <div className="row">
              <label className="grow">
                User
                <input spellCheck={false} value={t.user} onChange={(e) => setTarget({ user: e.target.value })} />
              </label>
              <label className="grow">
                Password
                <input
                  type="password"
                  value={password ?? ""}
                  placeholder={hasSaved && password === null ? "saved in Keychain" : ""}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </label>
            </div>
            <div className="row">
              <label className="grow">
                Database
                <input spellCheck={false} value={t.database} onChange={(e) => setTarget({ database: e.target.value })} />
              </label>
              <label>
                SSL
                <select value={t.ssl} onChange={(e) => setTarget({ ssl: e.target.value as SslMode })}>
                  <option value="disable">disable</option>
                  <option value="prefer">prefer</option>
                  <option value="require">require</option>
                </select>
              </label>
            </div>
          </>
        ) : (
          <div className="row">
            <label className="grow">
              File
              <input spellCheck={false} value={t.path} onChange={(e) => setTarget({ path: e.target.value })} />
            </label>
            <button
              type="button"
              className="align-end"
              onClick={async () => {
                const path = await open({ multiple: false, directory: false });
                if (typeof path === "string") setTarget({ path });
              }}
            >
              Choose…
            </button>
          </div>
        )}

        <label className="check">
          <input type="checkbox" checked={conn.agent} onChange={(e) => setConn({ ...conn, agent: e.target.checked })} />
          <span>
            Agents can query
            <span className="hint-inline">
              {" "}
              — read-only, through Fabio’s MCP server. Every statement shows up under Agent.
            </span>
          </span>
        </label>

        {status && <p className={status.ok ? "ok" : "error"}>{status.text}</p>}

        <div className="actions">
          {initial?.id && (
            <button type="button" className="danger" onClick={remove}>
              Delete
            </button>
          )}
          <span className="grow" />
          <button type="button" onClick={test} disabled={busy}>
            {busy ? "Testing…" : "Test"}
          </button>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="primary">
            Save
          </button>
        </div>
      </form>
    </div>
  );
}
