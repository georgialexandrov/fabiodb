import { useState } from "react";
import { describe } from "./ConnectionSwitcher";
import { api, fileName, plural, type Discovered, type Discovery, type SavedConnection, type Target } from "./api";

type Props = {
  folder: string;
  discovery: Discovery;
  existing: SavedConnection[];
  onSaved: (added: SavedConnection[]) => void;
  onClose: () => void;
};

/** Same database as a saved connection: same file, or same server, user and database. */
function sameTarget(a: Target, b: Target) {
  if (a.engine === "sqlite" || b.engine === "sqlite") {
    return a.engine === "sqlite" && b.engine === "sqlite" && a.path === b.path;
  }
  const local = (h: string) => (h === "127.0.0.1" || h === "::1" ? "localhost" : h);
  return local(a.host) === local(b.host) && a.port === b.port && a.user === b.user && a.database === b.database;
}

/** What a folder holds, ticked unless it's saved already; Save adds the ticked ones. */
export function DiscoverDialog({ folder, discovery, existing, onSaved, onClose }: Props) {
  const saved = (d: Discovered) => existing.some((c) => sameTarget(c.target, d.target));
  const [picked, setPicked] = useState(() => new Set(discovery.found.flatMap((d, i) => (saved(d) ? [] : [i]))));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const group = fileName(folder);

  async function save() {
    setBusy(true);
    try {
      const added: SavedConnection[] = [];
      for (const i of picked) {
        const d = discovery.found[i];
        const password = d.target.engine === "postgres" ? d.target.password : null;
        // The password goes to the Keychain; the store never writes it.
        added.push(await api.saveConnection({ id: "", name: d.name, target: d.target, agent: false, group }, password));
      }
      onSaved(added);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  const toggle = (i: number) =>
    setPicked((p) => {
      const next = new Set(p);
      if (!next.delete(i)) next.add(i);
      return next;
    });

  return (
    <div className="modal-backdrop" onMouseDown={() => !busy && onClose()}>
      <div className="modal wide" onMouseDown={(e) => e.stopPropagation()} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>Databases in {group}</h2>
        {discovery.found.length === 0 ? (
          <p className="muted">
            No Postgres service in a Compose file here, and no SQLite file. Fabio reads compose.yaml or docker-compose.yml
            in this folder.
          </p>
        ) : (
          <div className="discover-list">
            {discovery.found.map((d, i) => (
              <label key={i} className="discover-item">
                <input type="checkbox" checked={picked.has(i)} onChange={() => toggle(i)} />
                <span className={`engine ${d.target.engine}`}>{d.target.engine === "postgres" ? "PG" : "SQ"}</span>
                <span className="switcher-text">
                  <span className="ellipsis">
                    {d.name}
                    {saved(d) && <span className="muted"> · saved already</span>}
                  </span>
                  <span className="muted ellipsis switcher-sub">
                    {d.target.engine === "sqlite" ? fileName(d.source) : `${describe({ id: "", name: d.name, target: d.target, agent: false })} · ${fileName(d.source)}`}
                  </span>
                  {d.note && <span className="discover-note">{d.note}</span>}
                </span>
              </label>
            ))}
          </div>
        )}
        {discovery.problems.map((p) => (
          <p key={p} className="error">
            {p}
          </p>
        ))}
        {error && <p className="error">{error}</p>}
        <div className="actions">
          <span className="muted">Grouped under “{group}”.</span>
          <span className="grow" />
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="primary" onClick={save} disabled={busy || picked.size === 0}>
            {busy ? "Saving…" : `Add ${plural(picked.size, "connection")}`}
          </button>
        </div>
      </div>
    </div>
  );
}
