import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { AgentView, explainMode, formatMs } from "./AgentView";
import { CommandPalette, type Command } from "./CommandPalette";
import { ConnectionForm } from "./ConnectionForm";
import { ConnectionSwitcher, describe, markUsed } from "./ConnectionSwitcher";
import { InsightsView } from "./InsightsView";
import { QueryTab } from "./QueryTab";
import { TableView } from "./TableView";
import { applyTheme, nextTheme, savedTheme, THEME_LABELS, type Theme } from "./theme";
import {
  api,
  compactCount,
  sameRelation,
  type AuditEntry,
  type CompletionTable,
  type Relation,
  type RelationRef,
  type SavedConnection,
  type Snippet,
} from "./api";

const SQLITE_EXTENSIONS = /\.(db|sqlite|sqlite3|db3)$/i;

type Tab =
  | { id: string; connectionId: string; kind: "table"; relation: RelationRef }
  | { id: string; connectionId: string; kind: "query"; title: string; sql: string; autorun?: Autorun }
  | { id: string; connectionId: string; kind: "insights" }
  | { id: string; connectionId: string; kind: "agent" };

type Autorun = "run" | "explain" | "analyze";

/** The sidebar mentions agent activity this recent. */
const AGENT_RECENT_MS = 10 * 60_000;

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
  const [agentLog, setAgentLog] = useState<AuditEntry[]>([]);
  const lastAgentId = useRef(0);
  const [theme, setTheme] = useState<Theme>(savedTheme);
  const [palette, setPalette] = useState(false);
  const [switcher, setSwitcher] = useState(false);
  const [snippets, setSnippets] = useState<Snippet[]>([]);

  // Snippets are saved from query tabs; read them fresh whenever ⌘K opens.
  useEffect(() => {
    if (palette) api.listSnippets().then(setSnippets, () => {});
  }, [palette]);

  useEffect(() => applyTheme(theme), [theme]);

  useEffect(() => {
    api.listConnections().then(setConnections, (e) => setError(String(e)));
  }, []);

  // The MCP server writes agent statements to the shared audit log from its own
  // process; poll it while the window is visible. Idle when hidden.
  useEffect(() => {
    async function poll() {
      if (document.hidden) return;
      const fresh = await api.agentActivity(lastAgentId.current).catch(() => []);
      if (fresh.length === 0) return;
      lastAgentId.current = fresh[0].id;
      setAgentLog((log) => [...fresh, ...log].slice(0, 1000));
    }
    poll();
    const timer = setInterval(poll, 2000);
    document.addEventListener("visibilitychange", poll);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", poll);
    };
  }, []);

  async function activate(connection: SavedConnection) {
    setError(null);
    setSwitcher(false);
    markUsed(connection.id);
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

  function newQuery(connectionId = activeId, sql = "", autorun?: Autorun) {
    if (!connectionId) return;
    const n = tabs.filter((t) => t.connectionId === connectionId && t.kind === "query").length + 1;
    const tab: Tab = { id: `t${nextTab++}`, connectionId, kind: "query", title: `Query ${n}`, sql, autorun };
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

  async function openAgent(connectionId: string) {
    const conn = connections.find((c) => c.id === connectionId);
    if (!conn) return;
    if (connectionId !== activeId) await activate(conn);
    const existing = tabs.find((t) => t.connectionId === connectionId && t.kind === "agent");
    if (existing) return focusTab(existing);
    const tab: Tab = { id: `t${nextTab++}`, connectionId, kind: "agent" };
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
      else if (key === "l" && e.shiftKey) setTheme(nextTheme);
      else if (key === "k" && e.shiftKey) setSwitcher((s) => !s);
      else if (key === "k") setPalette((p) => !p);
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

  // "Agent ran 3 queries on chinook · 42 ms": the latest connection an agent used, lately.
  const agentSummary = useMemo(() => {
    const latest = agentLog[0];
    if (!latest || Date.now() - latest.at_ms > AGENT_RECENT_MS) return null;
    const conn = connections.find((c) => c.id === latest.connection_id);
    const burst = agentLog.filter((e) => e.connection_id === latest.connection_id && latest.at_ms - e.at_ms < AGENT_RECENT_MS);
    const ms = burst.reduce((sum, e) => sum + e.elapsed_ms, 0);
    const n = burst.length;
    return {
      connectionId: latest.connection_id,
      text: `Agent ran ${n} ${n === 1 ? "query" : "queries"} on ${conn?.name ?? "a deleted connection"} · ${formatMs(ms)}`,
    };
  }, [agentLog, connections]);

  const active = connections.find((c) => c.id === activeId) ?? null;

  const commands: Command[] = [
    ...(active
      ? [
          { id: "query", label: "New query", hint: active.name, shortcut: "⌘T", run: () => newQuery() },
          ...(active.target.engine === "postgres" ? [{ id: "insights", label: "Insights", hint: active.name, run: openInsights }] : []),
          { id: "agent", label: "Agent activity", hint: active.name, run: () => openAgent(active.id) },
          { id: "edit", label: `Edit connection “${active.name}”`, run: () => setEditing(active) },
        ]
      : []),
    ...(current ? [{ id: "close", label: "Close tab", shortcut: "⌘W", run: () => closeTab(current) }] : []),
    { id: "new", label: "New Postgres connection", shortcut: "⌘N", run: () => setEditing(null) },
    { id: "sqlite", label: "Open SQLite file", shortcut: "⌘O", run: () => chooseSqlite().catch((e) => setError(String(e))) },
    ...(["system", "light", "dark"] as Theme[])
      .filter((t) => t !== theme)
      .map((t) => ({ id: `theme-${t}`, label: `Theme: ${THEME_LABELS[t]}`, shortcut: "⇧⌘L", run: () => setTheme(t) })),
    ...(active
      ? snippets.map((sn) => ({
          id: `snippet-${sn.id}`,
          label: sn.name,
          hint: "snippet",
          run: () => newQuery(active.id, sn.sql),
        }))
      : []),
    ...connections
      .filter((c) => c.id !== activeId)
      .map((c) => ({
        id: `conn-${c.id}`,
        label: `${relations[c.id] ? "Switch to" : "Connect to"} ${c.name}`,
        hint: c.target.engine === "postgres" ? "Postgres" : "SQLite",
        run: () => activate(c),
      })),
    ...((activeId && relations[activeId]) || []).map((r) => ({
      id: `rel-${r.schema}.${r.name}`,
      label: r.schema === "public" || r.schema === "main" ? r.name : `${r.schema}.${r.name}`,
      hint: r.kind === "table" ? "table" : r.kind === "view" ? "view" : "materialized view",
      run: () => openTable(r),
    })),
  ];
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
          <button
            className="ghost theme-toggle"
            title={`Theme: ${THEME_LABELS[theme]}. Click for ${THEME_LABELS[nextTheme(theme)]} (⇧⌘L)`}
            onClick={() => setTheme(nextTheme)}
          >
            {theme === "system" ? "◐" : theme === "light" ? "○" : "●"}
          </button>
        </div>

        <div className="current-connection-wrap">
          {connections.length === 0 ? (
            <p className="hint">Add a Postgres connection, or drop a SQLite file anywhere.</p>
          ) : (
            <button className="current-connection" onClick={() => setSwitcher((v) => !v)} title="Switch connection (⇧⌘K)">
              {active ? (
                <>
                  <span className={`engine ${active.target.engine}`}>{active.target.engine === "postgres" ? "PG" : "SQ"}</span>
                  <span className="switcher-text">
                    <span className="ellipsis">{connecting ? "Connecting…" : active.name}</span>
                    <span className="muted ellipsis switcher-sub">{describe(active)}</span>
                  </span>
                </>
              ) : (
                <span className="grow muted">{connecting ? "Connecting…" : "Choose a connection"}</span>
              )}
              <span className="muted">▾</span>
            </button>
          )}
          {switcher && (
            <ConnectionSwitcher
              mode="popover"
              connections={connections}
              openIds={new Set(Object.keys(relations))}
              activeId={activeId}
              onPick={activate}
              onEdit={(c) => (setSwitcher(false), setEditing(c))}
              onNew={() => (setSwitcher(false), setEditing(null))}
              onOpenSqlite={() => (setSwitcher(false), chooseSqlite().catch((e) => setError(String(e))))}
              onClose={() => setSwitcher(false)}
            />
          )}
        </div>

        {active && (
          <div className="relations">
            <div className="relations-head">
              <input placeholder="Filter tables" value={search} onChange={(e) => setSearch(e.target.value)} spellCheck={false} />
              <button className="ghost" onClick={() => newQuery()} title="New query (⌘T)">
                SQL
              </button>
              {(active.agent || agentLog.some((e) => e.connection_id === active.id)) && (
                <button className="ghost" onClick={() => openAgent(active.id)} title="Statements agents ran on this connection">
                  Agent
                </button>
              )}
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

        {agentSummary && (
          <button className="agent-summary" onClick={() => openAgent(agentSummary.connectionId)}>
            {agentSummary.text}
          </button>
        )}
      </aside>

      <main className="content">
        {active && activeTabs.length > 0 && (
          <div className="tabs" data-tauri-drag-region>
            {activeTabs.map((t) => (
              <div key={t.id} className={`tab ${t.id === current ? "active" : ""}`} onClick={() => focusTab(t)} onAuxClick={() => closeTab(t.id)}>
                <span className={`tab-kind ${t.kind}`}>{t.kind === "query" ? "SQL" : ""}</span>
                <span className="ellipsis">
                  {t.kind === "query" ? t.title : t.kind === "insights" ? "Insights" : t.kind === "agent" ? "Agent" : t.relation.name}
                </span>
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
          if (t.kind === "agent") {
            return (
              <div key={t.id} className="tab-page" style={{ display: visible ? "flex" : "none" }}>
                <AgentView
                  entries={agentLog.filter((e) => e.connection_id === t.connectionId)}
                  onOpen={(e) => {
                    const { mode, sql } = explainMode(e.sql);
                    newQuery(t.connectionId, sql, mode);
                  }}
                />
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
                autorun={t.autorun}
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

        {!active && connections.length > 0 && (
          <div className="start">
            <ConnectionSwitcher
              mode="page"
              connections={connections}
              openIds={new Set(Object.keys(relations))}
              activeId={activeId}
              onPick={activate}
              onEdit={setEditing}
              onNew={() => setEditing(null)}
              onOpenSqlite={() => chooseSqlite().catch((e) => setError(String(e)))}
            />
          </div>
        )}

        {((!active && connections.length === 0) || (active && !currentTab)) && (
          <div className="empty">
            <img className="marmot" src="/fabio.png" alt="Fabio the marmot" width={96} height={96} />
            <p className="muted">{active ? "Pick a table, or ⌘T for a new query." : "Add a Postgres connection (⌘N), or drop a SQLite file here."}</p>
          </div>
        )}
      </main>

      {palette && <CommandPalette commands={commands} onClose={() => setPalette(false)} />}

      {editing !== undefined && (
        <ConnectionForm
          initial={editing}
          groups={[...new Set(connections.flatMap((c) => (c.group ? [c.group] : [])))].sort()}
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
