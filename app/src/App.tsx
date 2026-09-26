import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ConnectionForm } from "./ConnectionForm";
import { InsightsView } from "./InsightsView";
import { QueryTab } from "./QueryTab";
import { TableView } from "./TableView";
import {
  api,
  compactCount,
  sameRelation,
  type CompletionTable,
  type Relation,
  type RelationRef,
  type SavedConnection,
} from "./api";

const SQLITE_EXTENSIONS = /\.(db|sqlite|sqlite3|db3)$/i;

type Tab =
  | { id: string; connectionId: string; kind: "table"; relation: RelationRef }
  | { id: string; connectionId: string; kind: "query"; title: string; sql: string }
  | { id: string; connectionId: string; kind: "insights" };

let nextTab = 1;

export default function App() {
  const [connections, setConnections] = useState<SavedConnection[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [relations, setRelations] = useState<Record<string, Relation[]>>({});
  const [schemas, setSchemas] = useState<Record<string, CompletionTable[]>>({});
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<Record<string, string | null>>({});
  const [editing, setEditing] = useState<SavedConnection | null | undefined>(undefined);
  const [search, setSearch] = useState("");
  const [connecting, setConnecting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.listConnections().then(setConnections, (e) => setError(String(e)));
  }, []);

  async function activate(connection: SavedConnection) {
    setError(null);
    if (relations[connection.id]) {
      setActiveId(connection.id);
      return;
    }
    setConnecting(connection.id);
    try {
      const rels = await api.connect(connection.id);
      setRelations((r) => ({ ...r, [connection.id]: rels }));
      setActiveId(connection.id);
      setSearch("");
      api.completionSchema(connection.id).then((s) => setSchemas((all) => ({ ...all, [connection.id]: s })), () => {});
    } catch (e) {
      setError(`${connection.name}: ${e}`);
    } finally {
      setConnecting(null);
    }
  }

  function focusTab(tab: Tab) {
    setActiveTab((a) => ({ ...a, [tab.connectionId]: tab.id }));
  }

  function openTable(relation: RelationRef) {
    if (!activeId) return;
    const existing = tabs.find((t) => t.connectionId === activeId && t.kind === "table" && sameRelation(t.relation, relation));
    if (existing) return focusTab(existing);
    const tab: Tab = { id: `t${nextTab++}`, connectionId: activeId, kind: "table", relation };
    setTabs((all) => [...all, tab]);
    focusTab(tab);
  }

  function newQuery(connectionId = activeId, sql = "") {
    if (!connectionId) return;
    const n = tabs.filter((t) => t.connectionId === connectionId && t.kind === "query").length + 1;
    const tab: Tab = { id: `t${nextTab++}`, connectionId, kind: "query", title: `Query ${n}`, sql };
    setTabs((all) => [...all, tab]);
    focusTab(tab);
  }

  function openInsights() {
    if (!activeId) return;
    const existing = tabs.find((t) => t.connectionId === activeId && t.kind === "insights");
    if (existing) return focusTab(existing);
    const tab: Tab = { id: `t${nextTab++}`, connectionId: activeId, kind: "insights" };
    setTabs((all) => [...all, tab]);
    focusTab(tab);
  }

  function closeTab(id: string) {
    const tab = tabs.find((t) => t.id === id);
    if (!tab) return;
    const siblings = tabs.filter((t) => t.connectionId === tab.connectionId);
    const index = siblings.findIndex((t) => t.id === id);
    const next = siblings[index + 1] ?? siblings[index - 1] ?? null;
    setTabs((all) => all.filter((t) => t.id !== id));
    setActiveTab((a) => ({ ...a, [tab.connectionId]: a[tab.connectionId] === id ? (next?.id ?? null) : a[tab.connectionId] }));
  }

  function forget(connectionId: string) {
    setTabs((all) => all.filter((t) => t.connectionId !== connectionId));
    setRelations(({ [connectionId]: _, ...rest }) => rest);
    if (connectionId === activeId) setActiveId(null);
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

  const current = activeId ? (activeTab[activeId] ?? null) : null;

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "o") chooseSqlite().catch((err) => setError(String(err)));
      else if (key === "n") setEditing(null);
      else if (key === "t") newQuery();
      else if (key === "w" && current) closeTab(current);
      else return;
      e.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const grouped = useMemo(() => {
    const q = search.toLowerCase();
    const groups = new Map<string, Relation[]>();
    for (const r of (activeId && relations[activeId]) || []) {
      if (q && !r.name.toLowerCase().includes(q)) continue;
      groups.set(r.schema, [...(groups.get(r.schema) ?? []), r]);
    }
    return groups;
  }, [relations, activeId, search]);

  const active = connections.find((c) => c.id === activeId) ?? null;
  const activeTabs = tabs.filter((t) => t.connectionId === activeId);
  const currentTab = tabs.find((t) => t.id === current) ?? null;

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
              {relations[c.id] && <span className="dot" title="Connected" />}
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
              <button className="ghost" onClick={() => newQuery()} title="New query (⌘T)">
                SQL
              </button>
              {active.target.engine === "postgres" && (
                <button className="ghost" onClick={openInsights} title="What the server is doing, and where the time went">
                  Insights
                </button>
              )}
            </div>
            {[...grouped].map(([schema, rels]) => (
              <div key={schema}>
                {grouped.size > 1 || (schema !== "public" && schema !== "main") ? <div className="schema">{schema}</div> : null}
                {rels.map((r) => (
                  <div
                    key={r.name}
                    className={`relation ${r.kind} ${currentTab?.kind === "table" && sameRelation(r, currentTab.relation) ? "active" : ""}`}
                    onClick={() => openTable(r)}
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
        {active && activeTabs.length > 0 && (
          <div className="tabs" data-tauri-drag-region>
            {activeTabs.map((t) => (
              <div key={t.id} className={`tab ${t.id === current ? "active" : ""}`} onClick={() => focusTab(t)} onAuxClick={() => closeTab(t.id)}>
                <span className={`tab-kind ${t.kind}`}>{t.kind === "query" ? "SQL" : ""}</span>
                <span className="ellipsis">{t.kind === "query" ? t.title : t.kind === "insights" ? "Insights" : t.relation.name}</span>
                <button
                  className="ghost tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(t.id);
                  }}
                >
                  ✕
                </button>
              </div>
            ))}
            <button className="ghost new-tab" onClick={() => newQuery()} title="New query (⌘T)">
              +
            </button>
          </div>
        )}

        {error && (
          <p className="error banner" onClick={() => setError(null)}>
            {error}
          </p>
        )}

        {/* Every open tab stays mounted, so switching keeps scroll, filters and results. */}
        {tabs.map((t) => {
          const conn = connections.find((c) => c.id === t.connectionId);
          const visible = t.id === current && t.connectionId === activeId;
          if (!conn) return null;
          if (t.kind === "insights") {
            return (
              <div key={t.id} className="tab-page" style={{ display: visible ? "flex" : "none" }}>
                <InsightsView connectionId={t.connectionId} visible={visible} onOpenQuery={(sql) => newQuery(t.connectionId, sql)} />
              </div>
            );
          }
          if (t.kind === "query") {
            return (
              <QueryTab
                key={t.id}
                connectionId={t.connectionId}
                engine={conn.target.engine}
                schema={schemas[t.connectionId] ?? []}
                sql={t.sql}
                onSqlChange={(sql) => setTabs((all) => all.map((x) => (x.id === t.id ? { ...x, sql } : x)))}
                visible={visible}
              />
            );
          }
          const relation = relations[t.connectionId]?.find((r) => sameRelation(r, t.relation));
          if (!relation) return null;
          return (
            <div key={t.id} className="tab-page" style={{ display: visible ? "flex" : "none" }}>
              <TableView connectionId={t.connectionId} relation={relation} onOpen={openTable} />
            </div>
          );
        })}

        {(!active || !currentTab) && (
          <div className="empty">
            <img className="marmot" src="/fabio.png" alt="Fabio the marmot" width={96} height={96} />
            <p className="muted">{active ? "Pick a table, or ⌘T for a new query." : "Pick a connection, or drop a SQLite file here."}</p>
          </div>
        )}
      </main>

      {editing !== undefined && (
        <ConnectionForm
          initial={editing}
          onClose={() => setEditing(undefined)}
          onSaved={async (c) => {
            setEditing(undefined);
            forget(c.id);
            setConnections(await api.listConnections());
            await activate(c);
          }}
          onDeleted={async (id) => {
            setEditing(undefined);
            forget(id);
            setConnections(await api.listConnections());
          }}
        />
      )}
    </div>
  );
}
