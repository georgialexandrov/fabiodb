import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ConnectionForm } from "./ConnectionForm";
import { QueryView } from "./QueryView";
import { TableView } from "./TableView";
import { api, compactCount, sameRelation, type Relation, type RelationRef, type SavedConnection } from "./api";

const SQLITE_EXTENSIONS = /\.(db|sqlite|sqlite3|db3)$/i;

export default function App() {
  const [connections, setConnections] = useState<SavedConnection[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [relations, setRelations] = useState<Relation[]>([]);
  const [selected, setSelected] = useState<RelationRef | null>(null);
  const [view, setView] = useState<"table" | "sql">("table");
  const [editing, setEditing] = useState<SavedConnection | null | undefined>(undefined);
  const [search, setSearch] = useState("");
  const [connecting, setConnecting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.listConnections().then(setConnections, (e) => setError(String(e)));
  }, []);

  async function activate(connection: SavedConnection) {
    setConnecting(connection.id);
    setError(null);
    try {
      const rels = await api.connect(connection.id);
      setActiveId(connection.id);
      setRelations(rels);
      setSelected(null);
      setSearch("");
      setView("table");
    } catch (e) {
      setError(`${connection.name}: ${e}`);
    } finally {
      setConnecting(null);
    }
  }

  async function openSqlite(path: string) {
    const conn = await api.openSqliteFile(path);
    setConnections(await api.listConnections());
    await activate(conn);
  }

  async function chooseSqlite() {
    const path = await open({
      multiple: false,
      filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3", "db3"] }, { name: "All files", extensions: ["*"] }],
    });
    if (typeof path === "string") await openSqlite(path);
  }

  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type !== "drop") return;
      const path = e.payload.paths.find((p) => SQLITE_EXTENSIONS.test(p));
      if (path) openSqlite(path).catch((err) => setError(String(err)));
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "o") {
        e.preventDefault();
        chooseSqlite().catch((err) => setError(String(err)));
      } else if (e.key === "n") {
        e.preventDefault();
        setEditing(null);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const grouped = useMemo(() => {
    const q = search.toLowerCase();
    const groups = new Map<string, Relation[]>();
    for (const r of relations) {
      if (q && !r.name.toLowerCase().includes(q)) continue;
      groups.set(r.schema, [...(groups.get(r.schema) ?? []), r]);
    }
    return groups;
  }, [relations, search]);

  const active = connections.find((c) => c.id === activeId) ?? null;
  const selectedRelation = relations.find((r) => sameRelation(r, selected)) ?? null;

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="sidebar-head" data-tauri-drag-region>
          <span className="brand">Fabio</span>
          <span className="grow" />
          <button className="ghost" title="New Postgres connection (⌘N)" onClick={() => setEditing(null)}>
            +
          </button>
          <button className="ghost" title="Open SQLite file (⌘O)" onClick={() => chooseSqlite().catch((e) => setError(String(e)))}>
            ⌘O
          </button>
        </div>

        <nav className="connections">
          {connections.length === 0 && <p className="hint">Add a Postgres connection, or drop a SQLite file anywhere.</p>}
          {connections.map((c) => (
            <div
              key={c.id}
              className={`connection ${c.id === activeId ? "active" : ""}`}
              onClick={() => activate(c)}
              title={c.target.engine === "sqlite" ? c.target.path : `${c.target.user}@${c.target.host}:${c.target.port}/${c.target.database}`}
            >
              <span className={`engine ${c.target.engine}`}>{c.target.engine === "postgres" ? "PG" : "SQ"}</span>
              <span className="grow ellipsis">{connecting === c.id ? "Connecting…" : c.name}</span>
              <button
                className="ghost edit"
                onClick={(e) => {
                  e.stopPropagation();
                  setEditing(c);
                }}
              >
                ⋯
              </button>
            </div>
          ))}
        </nav>

        {active && (
          <div className="relations">
            <div className="relations-head">
              <input placeholder="Filter tables" value={search} onChange={(e) => setSearch(e.target.value)} spellCheck={false} />
              <button className={`ghost ${view === "sql" ? "on" : ""}`} onClick={() => setView("sql")} title="SQL">
                SQL
              </button>
            </div>
            {[...grouped].map(([schema, rels]) => (
              <div key={schema}>
                {grouped.size > 1 || schema !== "public" ? <div className="schema">{schema}</div> : null}
                {rels.map((r) => (
                  <div
                    key={r.name}
                    className={`relation ${r.kind} ${view === "table" && sameRelation(r, selected) ? "active" : ""}`}
                    onClick={() => {
                      setSelected(r);
                      setView("table");
                    }}
                  >
                    <span className="grow ellipsis">{r.name}</span>
                    {r.estimated_rows != null && <span className="count">{compactCount(r.estimated_rows)}</span>}
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </aside>

      <main className="content">
        {error && (
          <p className="error banner" onClick={() => setError(null)}>
            {error}
          </p>
        )}
        {active && view === "sql" && <QueryView key={active.id} connectionId={active.id} />}
        {active && view === "table" && selectedRelation && (
          <TableView
            connectionId={active.id}
            relation={selectedRelation}
            onOpen={(r) => {
              setSelected(r);
              setView("table");
            }}
          />
        )}
        {(!active || (view === "table" && !selectedRelation)) && (
          <div className="empty">
            <div className="marmot">Fabio</div>
            <p className="muted">{active ? "Pick a table." : "Pick a connection, or drop a SQLite file here."}</p>
          </div>
        )}
      </main>

      {editing !== undefined && (
        <ConnectionForm
          initial={editing}
          onClose={() => setEditing(undefined)}
          onSaved={async (c) => {
            setEditing(undefined);
            setConnections(await api.listConnections());
            await activate(c);
          }}
          onDeleted={async (id) => {
            setEditing(undefined);
            if (id === activeId) {
              setActiveId(null);
              setRelations([]);
            }
            setConnections(await api.listConnections());
          }}
        />
      )}
    </div>
  );
}
