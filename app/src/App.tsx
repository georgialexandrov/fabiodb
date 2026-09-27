import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { AgentView, explainMode, formatMs } from "./AgentView";
import type { Command } from "./CommandPalette";
import { ConnectionSwitcher, describe, markUsed } from "./ConnectionSwitcher";
import { lazyComponent } from "./lazy";
import { applyTheme, nextTheme, savedTheme, THEME_LABELS, type Theme } from "./theme";
import {
  api,
  baseId,
  workspaceDatabase,
  workspaceId,
  compactCount,
  sameRelation,
  type AuditEntry,
  type Discovery,
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
  | { id: string; connectionId: string; kind: "diagram" }
  | { id: string; connectionId: string; kind: "agent" };

type Autorun = "run" | "explain" | "analyze";

/** The sidebar mentions agent activity this recent. */
const AGENT_RECENT_MS = 10 * 60_000;

// Loaded when first used, so the first paint only waits for the shell.
const CommandPalette = lazyComponent(() => import("./CommandPalette").then((m) => m.CommandPalette));
const ConnectionForm = lazyComponent(() => import("./ConnectionForm").then((m) => m.ConnectionForm));
const DiscoverDialog = lazyComponent(() => import("./DiscoverDialog").then((m) => m.DiscoverDialog));
const DiagramView = lazyComponent(() => import("./DiagramView").then((m) => m.DiagramView));
const InsightsView = lazyComponent(() => import("./InsightsView").then((m) => m.InsightsView));
const QueryTab = lazyComponent(() => import("./QueryTab").then((m) => m.QueryTab));
const TableView = lazyComponent(() => import("./TableView").then((m) => m.TableView));

let nextTab = 1;

/** Tabs and the workspace in use, kept between launches ("where you left it"). */
const LAYOUT_KEY = "fabio.layout";
type Layout = { activeId: string | null; tabs: Tab[]; activeTab: Record<string, string | null> };

function savedLayout(): Layout | null {
  try {
    const layout: Layout | null = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    if (!layout || !Array.isArray(layout.tabs)) return null;
    for (const t of layout.tabs) nextTab = Math.max(nextTab, Number(t.id.slice(1)) + 1 || 0);
    return layout;
  } catch {
    return null;
  }
}

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
  // ⌘D: the databases on the current server, to switch between.
  const [databases, setDatabases] = useState<string[] | null>(null);
  const [discovered, setDiscovered] = useState<{ folder: string; discovery: Discovery } | null>(null);
  const [snippets, setSnippets] = useState<Snippet[]>([]);

  // Snippets are saved from query tabs; read them fresh whenever ⌘K opens.
  useEffect(() => {
    if (palette) api.listSnippets().then(setSnippets, () => {});
  }, [palette]);

  useEffect(() => applyTheme(theme), [theme]);

  useEffect(() => {
    api.listConnections().then(
      (all) => {
        setConnections(all);
        // Put tabs back, and reconnect the workspace that was in use; the
        // others connect when you switch to them.
        const layout = savedLayout();
        if (!layout) return;
        const known = (id: string) => all.some((c) => c.id === baseId(id));
        setTabs(layout.tabs.filter((t) => known(t.connectionId)).map((t) => (t.kind === "query" ? { ...t, autorun: undefined } : t)));
        setActiveTab(layout.activeTab ?? {});
        const conn = layout.activeId && known(layout.activeId) ? all.find((c) => c.id === baseId(layout.activeId!)) : null;
        if (conn) activate(conn, workspaceDatabase(layout.activeId!));
      },
      (e) => setError(String(e)),
    );
  }, []);

  useEffect(() => {
    // Only once something has loaded, so a failed start doesn't wipe the saved layout.
    if (connections.length === 0) return;
    try {
      localStorage.setItem(LAYOUT_KEY, JSON.stringify({ activeId, tabs, activeTab } satisfies Layout));
    } catch {
      // Not remembered this time.
    }
  }, [activeId, tabs, activeTab, connections.length]);

  // The MCP server writes agent statements to the shared audit log from its own
  // process; poll it while the window is visible. Idle when hidden.
  useEffect(() => {
    async function poll() {
      if (document.hidden) return;
      const fresh = await api.agentActivity(lastAgentId.current).catch(() => []);
      if (fresh.length === 0) return;
      lastAgentId.current = fresh[0].id;
      setAgentLog((log) => [...fresh, ...log].slice(0, 1000));
      // The MCP server saved a connection: show it in the switcher.
      if (fresh.some((e) => e.sql.startsWith("-- Agent added connection"))) {
        api.listConnections().then(setConnections, () => {});
      }
    }
    poll();
    const timer = setInterval(poll, 2000);
    document.addEventListener("visibilitychange", poll);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", poll);
    };
  }, []);

  /** Opens (or returns to) a connection's workspace; `database` picks another one on its server. */
  async function activate(connection: SavedConnection, database: string | null = null) {
    setError(null);
    setSwitcher(false);
    markUsed(connection.id);
    const id = workspaceId(connection, database);
    if (relations[id]) {
      setActiveId(id);
      return;
    }
    setConnecting(connection.id);
    try {
      const rels = await api.connect(id);
      setRelations((r) => ({ ...r, [id]: rels }));
      setActiveId(id);
      setSearch("");
      api.completionSchema(id).then((s) => setSchemas((all) => ({ ...all, [id]: s })), () => {});
    } catch (e) {
      setError(`${connection.name}${database ? ` / ${database}` : ""}: ${e}`);
    } finally {
      setConnecting(null);
    }
  }

  async function pickDatabase() {
    if (!activeId || !active || active.target.engine !== "postgres") return;
    try {
      setDatabases(await api.databases(activeId));
    } catch (e) {
      setError(String(e));
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

  function openDiagram() {
    if (!activeId) return;
    const existing = tabs.find((t) => t.connectionId === activeId && t.kind === "diagram");
    if (existing) return focusTab(existing);
    const tab: Tab = { id: `t${nextTab++}`, connectionId: activeId, kind: "diagram" };
    setTabs((all) => [...all, tab]);
    focusTab(tab);
  }

  async function openAgent(connectionId: string) {
    const conn = connections.find((c) => c.id === baseId(connectionId));
    if (!conn) return;
    if (connectionId !== activeId) await activate(conn, workspaceDatabase(connectionId));
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

  /** Drops a connection's workspaces (all its databases), after an edit or delete. */
  function forget(connectionId: string) {
    const mine = (id: string) => baseId(id) === connectionId;
    setTabs((all) => all.filter((t) => !mine(t.connectionId)));
    setRelations((r) => Object.fromEntries(Object.entries(r).filter(([id]) => !mine(id))));
    if (activeId && mine(activeId)) setActiveId(null);
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

  /** A project folder: offer the Compose services and SQLite files in it. */
  async function scanFolder() {
    setSwitcher(false);
    const folder = await open({ directory: true, multiple: false });
    if (typeof folder !== "string") return;
    setDiscovered({ folder, discovery: await api.discoverFolder(folder) });
  }

  // Menu clicks (keyboard shortcuts reach the page first and never get here).
  const onMenu = useRef<(id: string) => void>(() => {});
  onMenu.current = (id: string) => {
    const actions: Record<string, () => void> = {
      "new-connection": () => setEditing(null),
      "open-sqlite": () => chooseSqlite().catch((e) => setError(String(e))),
      "scan-folder": () => scanFolder().catch((e) => setError(String(e))),
      "new-query": () => newQuery(),
      "close-tab": () => current && closeTab(current),
      palette: () => setPalette(true),
      switcher: () => setSwitcher(true),
      databases: () => pickDatabase(),
      diagram: () => openDiagram(),
      "row-pane": () => document.dispatchEvent(new CustomEvent("fabio-row-pane")),
      theme: () => setTheme(nextTheme),
    };
    actions[id]?.();
  };
  useEffect(() => {
    const unlisten = listen<string>("menu", (e) => onMenu.current(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

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
      // Keys the editor already used (its ⌘D selects the next match) aren't app shortcuts.
      if (!(e.metaKey || e.ctrlKey) || e.defaultPrevented) return;
      const key = e.key.toLowerCase();
      if (key === "o") chooseSqlite().catch((err) => setError(String(err)));
      else if (key === "n") setEditing(null);
      else if (key === "t") newQuery();
      else if (key === "w" && current) closeTab(current);
      else if (key === "l" && e.shiftKey) setTheme(nextTheme);
      else if (key === "k" && e.shiftKey) setSwitcher((s) => !s);
      else if (key === "d" && e.shiftKey) openDiagram();
      else if (key === "d") pickDatabase();
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

  const active = (activeId && connections.find((c) => c.id === baseId(activeId))) || null;
  const activeDatabase = activeId ? workspaceDatabase(activeId) : null;

  const commands: Command[] = [
    ...(active
      ? [
          { id: "query", label: "New query", hint: active.name, shortcut: "⌘T", run: () => newQuery() },
          { id: "diagram", label: "Diagram", hint: active.name, run: openDiagram },
          ...(active.target.engine === "postgres" ? [{ id: "insights", label: "Insights", hint: active.name, run: openInsights }] : []),
          { id: "agent", label: "Agent activity", hint: active.name, run: () => openAgent(active.id) },
          { id: "edit", label: `Edit connection “${active.name}”`, run: () => setEditing(active) },
          ...(active.target.engine === "postgres"
            ? [{ id: "databases", label: "Switch database…", hint: active.name, shortcut: "⌘D", run: pickDatabase }]
            : []),
        ]
      : []),
    ...(current ? [{ id: "close", label: "Close tab", shortcut: "⌘W", run: () => closeTab(current) }] : []),
    { id: "new", label: "New Postgres connection", shortcut: "⌘N", run: () => setEditing(null) },
    { id: "sqlite", label: "Open SQLite file", shortcut: "⌘O", run: () => chooseSqlite().catch((e) => setError(String(e))) },
    { id: "docker", label: "Find databases in a folder (Docker Compose, SQLite)", run: () => scanFolder().catch((e) => setError(String(e))) },
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
                    <span className="muted ellipsis switcher-sub">
                      {activeDatabase && active.target.engine === "postgres"
                        ? `${active.target.user}@${active.target.host}:${active.target.port}/${activeDatabase}`
                        : describe(active)}
                    </span>
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
              openIds={new Set(Object.keys(relations).map(baseId))}
              activeId={activeId && baseId(activeId)}
              onPick={(c) => activate(c)}
              onEdit={(c) => (setSwitcher(false), setEditing(c))}
              onNew={() => (setSwitcher(false), setEditing(null))}
              onOpenSqlite={() => (setSwitcher(false), chooseSqlite().catch((e) => setError(String(e))))}
              onScanFolder={() => scanFolder().catch((e) => setError(String(e)))}
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
              <button className="ghost" onClick={openDiagram} title="Tables and their references (⇧⌘D)">
                Diagram
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
                  {t.kind === "query"
                    ? t.title
                    : t.kind === "insights"
                      ? "Insights"
                      : t.kind === "agent"
                        ? "Agent"
                        : t.kind === "diagram"
                          ? "Diagram"
                          : t.relation.name}
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
          const conn = connections.find((c) => c.id === baseId(t.connectionId));
          const visible = t.id === current && t.connectionId === activeId;
          // Restored tabs wait until their workspace is connected.
          if (!conn || !relations[t.connectionId]) return null;
          if (t.kind === "insights") {
            return (
              <div key={t.id} className="tab-page" style={{ display: visible ? "flex" : "none" }}>
                <InsightsView connectionId={t.connectionId} visible={visible} onOpenQuery={(sql) => newQuery(t.connectionId, sql)} />
              </div>
            );
          }
          if (t.kind === "diagram") {
            return (
              <div key={t.id} className="tab-page" style={{ display: visible ? "flex" : "none" }}>
                <DiagramView connectionId={t.connectionId} visible={visible} onOpen={openTable} />
              </div>
            );
          }
          if (t.kind === "agent") {
            return (
              <div key={t.id} className="tab-page" style={{ display: visible ? "flex" : "none" }}>
                <AgentView
                  entries={agentLog.filter((e) => e.connection_id === baseId(t.connectionId))}
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
              openIds={new Set(Object.keys(relations).map(baseId))}
              activeId={null}
              onPick={(c) => activate(c)}
              onEdit={setEditing}
              onNew={() => setEditing(null)}
              onOpenSqlite={() => chooseSqlite().catch((e) => setError(String(e)))}
              onScanFolder={() => scanFolder().catch((e) => setError(String(e)))}
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
      {databases && active && (
        <CommandPalette
          placeholder={`Database on ${active.target.engine === "postgres" ? active.target.host : active.name}`}
          commands={databases.map((d) => {
            const id = workspaceId(active, d);
            return {
              id: `db-${d}`,
              label: d,
              hint: id === activeId ? "current" : relations[id] ? "open" : undefined,
              run: () => activate(active, d),
            };
          })}
          onClose={() => setDatabases(null)}
        />
      )}

      {discovered && (
        <DiscoverDialog
          folder={discovered.folder}
          discovery={discovered.discovery}
          existing={connections}
          onClose={() => setDiscovered(null)}
          onSaved={async (added) => {
            setDiscovered(null);
            setConnections(await api.listConnections());
            if (added.length === 1) await activate(added[0]);
            else if (added.length > 1) setSwitcher(true);
          }}
        />
      )}

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
