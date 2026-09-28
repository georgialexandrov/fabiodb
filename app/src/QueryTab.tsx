import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { chooseFile, ExportMenu } from "./ExportMenu";
import { Grid } from "./Grid";
import { RowPaneButton } from "./rowPane";
import {
  api,
  fileName,
  plural,
  type AuditEntry,
  type CompletionTable,
  type ExportFormat,
  type Plan,
  type QueryError,
  type QueryResult,
  type Snippet,
} from "./api";
import { lazyComponent } from "./lazy";
import { PlanView } from "./PlanView";
import { splitStatements, statementAt, type Statement } from "./statements";
import type { EditorSnapshot } from "./SqlEditor";

const SqlEditor = lazy(() => import("./SqlEditor"));
const ChartView = lazyComponent(() => import("./ChartView").then((m) => m.default));

type Props = {
  connectionId: string;
  engine: "postgres" | "sqlite";
  schema: CompletionTable[];
  /** The schema picked in the sidebar. */
  activeSchema: string | null;
  sql: string;
  onSqlChange: (sql: string) => void;
  visible: boolean;
  /** Run or explain `sql` once the session is open (opening an agent statement). */
  autorun?: "run" | "explain" | "analyze";
};

type Failure = { message: string; statement?: Statement; position?: number | null };

export function QueryTab({ connectionId, engine, schema, activeSchema, sql, onSqlChange, visible, autorun }: Props) {
  const [session, setSession] = useState<string | null>(null);
  const [writable, setWritable] = useState(false);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [ran, setRan] = useState<{ count: number; ms: number } | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [plan, setPlan] = useState<{ sql: string; plan: Plan; previous: Plan | null } | null>(null);
  const [view, setView] = useState<"results" | "chart" | "plan">("results");
  const [history, setHistory] = useState<AuditEntry[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [panel, setPanel] = useState<"history" | "snippets">("history");
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  // Naming a snippet: the statement to save and the name typed so far.
  const [naming, setNaming] = useState<{ sql: string; name: string } | null>(null);
  const cancelled = useRef(false);

  // Each query tab has its own connection: its own session, transaction and write mode.
  useEffect(() => {
    let id: string | null = null;
    let gone = false;
    api.openSession(connectionId).then(
      (s) => (gone ? api.closeSession(s) : ((id = s), setSession(s))),
      (e) => setFailure({ message: String(e) }),
    );
    return () => {
      gone = true;
      if (id) api.closeSession(id);
    };
  }, [connectionId]);

  const autoran = useRef(false);
  useEffect(() => {
    if (!session || !autorun || autoran.current) return;
    autoran.current = true;
    const at = { text: sql, from: 0, to: 0, head: 0 };
    if (autorun === "run") run("statement", at);
    else explain(autorun === "analyze", at);
  }, [session]);

  const refreshHistory = () => api.history(connectionId, 200).then(setHistory, () => {});
  const refreshSnippets = () => api.listSnippets().then(setSnippets, () => {});
  useEffect(() => {
    if (showHistory) (panel === "history" ? refreshHistory : refreshSnippets)();
  }, [showHistory, panel]);

  async function saveSnippet() {
    if (!naming) return;
    const name = naming.name.trim();
    const existing = snippets.find((s) => s.name.toLowerCase() === name.toLowerCase());
    try {
      await api.saveSnippet({ id: existing?.id ?? "", name, sql: naming.sql });
      setNaming(null);
      setNotice(`${existing ? "Replaced" : "Saved"} snippet “${name}” · ⌘K finds it`);
      refreshSnippets();
    } catch (e) {
      setNotice(String(e));
    }
  }

  /** The selection, or the statement under the cursor. */
  function target(at: EditorSnapshot): Statement | undefined {
    if (at.from !== at.to) {
      const text = at.text.slice(at.from, at.to);
      const lead = text.length - text.trimStart().length;
      return { from: at.from + lead, to: at.to, text: text.trim().replace(/;\s*$/, "") };
    }
    return statementAt(splitStatements(at.text), at.head);
  }

  async function explain(analyze: boolean, at: EditorSnapshot) {
    const statement = target(at);
    if (!session || running || !statement) return;
    setRunning(true);
    setFailure(null);
    try {
      const p = await api.explain(session, statement.text, analyze);
      const same = plan && plan.sql.replace(/\s+/g, " ") === statement.text.replace(/\s+/g, " ");
      setPlan({ sql: statement.text, plan: p, previous: same && plan.plan.analyzed ? plan.plan : null });
      setView("plan");
    } catch (e) {
      const err = e as QueryError;
      setFailure({ message: err.message ?? String(e), statement, position: err.position });
    } finally {
      setRunning(false);
      if (showHistory) refreshHistory();
    }
  }

  async function run(mode: "statement" | "all", at: EditorSnapshot) {
    if (!session || running) return;
    // A re-run stays on the chart; only the plan belongs to the statement explained.
    setView((v) => (v === "plan" ? "results" : v));
    let statements: Statement[];
    if (at.from !== at.to) {
      // A selection runs exactly what's selected, split into statements.
      statements = splitStatements(at.text.slice(at.from, at.to)).map((s) => ({ ...s, from: s.from + at.from, to: s.to + at.from }));
    } else {
      const all = splitStatements(at.text);
      const current = statementAt(all, at.head);
      statements = mode === "all" ? all : current ? [current] : [];
    }
    if (statements.length === 0) return;

    setRunning(true);
    setFailure(null);
    setNotice(null);
    cancelled.current = false;
    let total = 0;
    let current = statements[0];
    try {
      for (const s of statements) {
        if (cancelled.current) break;
        current = s;
        const r = await api.runStatement(session, s.text);
        total += r.elapsed_ms;
        setResult(r);
      }
      setRan({ count: statements.length, ms: total });
    } catch (e) {
      const err = e as QueryError;
      setFailure({ message: err.message ?? String(e), statement: current, position: err.position });
    } finally {
      setRunning(false);
      if (showHistory) refreshHistory();
    }
  }

  async function format(at: EditorSnapshot) {
    const { format } = await import("sql-formatter");
    const opts = { language: engine === "postgres" ? "postgresql" : "sqlite", keywordCase: "preserve" } as const;
    try {
      if (at.from !== at.to) {
        onSqlChange(at.text.slice(0, at.from) + format(at.text.slice(at.from, at.to), opts) + at.text.slice(at.to));
      } else {
        onSqlChange(format(at.text, opts));
      }
    } catch (e) {
      setFailure({ message: `Can't format: ${e instanceof Error ? e.message.split("\n")[0] : e}` });
    }
  }

  // Query results have no column types, so values export as text.
  const resultColumns = () => result!.columns.map((name) => ({ name, data_type: "" }));

  async function saveResult(format: ExportFormat) {
    const path = await chooseFile("result", format);
    if (!path) return null;
    await api.exportRows(resultColumns(), result!.rows, format, null, path);
    return `Saved ${plural(result!.rows.length, "row")} to ${fileName(path)}`;
  }

  async function copyResult(format: ExportFormat, label: string) {
    await navigator.clipboard.writeText(await api.copyRows(resultColumns(), result!.rows, format, null));
    return `Copied ${plural(result!.rows.length, "row")} as ${label}`;
  }

  async function cancel() {
    if (!running || !session) return;
    cancelled.current = true;
    await api.cancel(session).catch(() => {});
  }

  async function toggleWrite() {
    if (!session) return;
    await api.setWriteMode(session, !writable);
    setWritable(!writable);
  }

  useEffect(() => {
    if (!visible) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && running) cancel();
      if ((e.metaKey || e.ctrlKey) && e.key === "y") {
        e.preventDefault();
        setShowHistory((s) => !s);
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "s" && sql.trim()) {
        e.preventDefault();
        refreshSnippets();
        setNaming({ sql: sql.trim(), name: "" });
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, running, session, sql]);

  // Postgres positions are 1-based characters into the statement; underline the word there.
  let errorRange: { from: number; to: number } | null = null;
  if (failure?.statement && failure.position) {
    const from = failure.statement.from + failure.position - 1;
    const word = /^[\w."]+/.exec(sql.slice(from))?.[0].length ?? 1;
    errorRange = { from, to: from + Math.max(1, word) };
  }

  return (
    <div className="query-tab" style={{ display: visible ? "flex" : "none" }}>
      <div className={`query-toolbar ${writable ? "writable" : ""}`}>
        <button className="primary" disabled={!session || running} onClick={() => document.dispatchEvent(new CustomEvent("fabio-run"))} title="Run statement (⌘↵)">
          Run
        </button>
        <button disabled={!session || running} onClick={() => document.dispatchEvent(new CustomEvent("fabio-explain"))} title="Explain: runs the statement, rolled back (⌘E). ⇧⌘E estimates without running.">
          Explain
        </button>
        {running && (
          <button onClick={cancel} title="Cancel (Esc)">
            Cancel
          </button>
        )}
        <span className="grow" />
        {writable && <span className="write-note">Write mode — statements can change data.</span>}
        {result && result.columns.length > 0 && view === "results" && (
          <ExportMenu
            onDone={setNotice}
            note={
              result.truncated
                ? `The ${result.rows.length.toLocaleString()} rows shown; the result was capped.`
                : `The ${plural(result.rows.length, "row")} shown.`
            }
            items={[
              { label: "CSV…", run: () => saveResult("csv") },
              { label: "CSV for spreadsheets…", run: () => saveResult("spreadsheet_csv") },
              { label: "JSON…", run: () => saveResult("json") },
              "separator",
              { label: "Copy as Markdown", run: () => copyResult("markdown", "Markdown") },
              { label: "Copy as CSV", run: () => copyResult("csv", "CSV") },
            ]}
          />
        )}
        <button className={writable ? "danger on" : "ghost"} onClick={toggleWrite} disabled={!session} title="Toggle write mode for this tab">
          {writable ? "Write" : "Read-only"}
        </button>
        <button className={`ghost ${showHistory ? "on" : ""}`} onClick={() => setShowHistory(!showHistory)} title="History (⌘Y)">
          History
        </button>
        {result && result.columns.length > 0 && view === "results" && <RowPaneButton />}
      </div>

      <div className="query-body">
        <div className="query-main">
          <div className="editor-host">
            <Suspense fallback={<div className="sql-editor" />}>
              <EditorWithRunEvent
                value={sql}
                onChange={onSqlChange}
                engine={engine}
                schema={schema}
                activeSchema={activeSchema}
                onRun={run}
                onFormat={format}
                onExplain={explain}
                onCancel={cancel}
                errorRange={errorRange}
                visible={visible}
              />
            </Suspense>
          </div>

          {failure && <p className="error">{failure.message}</p>}
          {result && result.columns.length > 0 && (
            <div className="segmented result-switch">
              <button className={view === "results" ? "on" : ""} onClick={() => setView("results")}>
                Results
              </button>
              <button className={view === "chart" ? "on" : ""} onClick={() => setView("chart")}>
                Chart
              </button>
              {plan && (
                <button className={view === "plan" ? "on" : ""} onClick={() => setView("plan")}>
                  Plan
                </button>
              )}
            </div>
          )}
          {view === "plan" && plan ? (
            <PlanView plan={plan.plan} previous={plan.previous} engine={engine} />
          ) : view === "chart" && result && result.columns.length > 0 ? (
            <ChartView columns={result.columns} rows={result.rows} />
          ) : (
            <>
          {result && !failure && result.truncated && (
            <p className="notice">Showing the first {result.rows.length.toLocaleString()} rows. Add a LIMIT or a filter to see the rest.</p>
          )}
          {result && result.columns.length > 0 ? (
            <Grid
              columns={result.columns.map((name) => ({ name, data_type: "" }))}
              rowCount={result.rows.length}
              row={(i) => result.rows[i]}
              sample={result.rows}
              onNotice={setNotice}
            />
          ) : (
            <div className="grid-wrap" />
          )}
            </>
          )}
        </div>

        {showHistory && (
          <aside className="history">
            <div className="segmented history-switch">
              <button className={panel === "history" ? "on" : ""} onClick={() => setPanel("history")}>
                History
              </button>
              <button className={panel === "snippets" ? "on" : ""} onClick={() => setPanel("snippets")}>
                Snippets
              </button>
            </div>
            {panel === "history" && history.length === 0 && <p className="hint">Statements you run here show up in this list.</p>}
            {panel === "history" &&
              history.map((h) => (
                <div key={h.id} className={`history-item ${h.error ? "failed" : ""}`} onClick={() => onSqlChange(h.sql)} title={h.error ?? h.sql}>
                  <div className="history-sql">{h.sql}</div>
                  <div className="history-meta">
                    {timeAgo(h.at_ms)} · {h.error ? "failed" : `${h.rows ?? 0} rows`} · {h.elapsed_ms.toFixed(0)} ms
                    {h.source === "agent" && " · agent"}
                  </div>
                </div>
              ))}
            {panel === "snippets" && snippets.length === 0 && <p className="hint">⌘S saves the editor’s text as a snippet.</p>}
            {panel === "snippets" &&
              snippets.map((sn) => (
                <div key={sn.id} className="history-item" onClick={() => onSqlChange(sn.sql)} title={sn.sql}>
                  <div className="snippet-head">
                    <span className="snippet-name ellipsis">{sn.name}</span>
                    <button
                      className="ghost"
                      title="Delete snippet"
                      onClick={(e) => {
                        e.stopPropagation();
                        api.deleteSnippet(sn.id).then(refreshSnippets);
                      }}
                    >
                      ✕
                    </button>
                  </div>
                  <div className="history-sql">{sn.sql}</div>
                </div>
              ))}
          </aside>
        )}
      </div>

      {naming && (
        <div className="modal-backdrop" onMouseDown={() => setNaming(null)}>
          <form
            className="modal"
            onMouseDown={(e) => e.stopPropagation()}
            onSubmit={(e) => {
              e.preventDefault();
              saveSnippet();
            }}
          >
            <h2>Save as snippet</h2>
            <label>
              Name
              <input
                autoFocus
                value={naming.name}
                onChange={(e) => setNaming({ ...naming, name: e.target.value })}
                onKeyDown={(e) => e.key === "Escape" && setNaming(null)}
              />
            </label>
            {snippets.some((s) => s.name.toLowerCase() === naming.name.trim().toLowerCase()) && (
              <p className="muted">Replaces the snippet with this name.</p>
            )}
            <pre className="sql-preview">{naming.sql}</pre>
            <div className="actions">
              <span className="grow" />
              <button type="button" onClick={() => setNaming(null)}>
                Cancel
              </button>
              <button type="submit" className="primary" disabled={!naming.name.trim()}>
                Save
              </button>
            </div>
          </form>
        </div>
      )}

      <footer className="status">
        {!session
          ? "Opening a session…"
          : running
            ? "Running… Esc to cancel"
            : failure
              ? "Failed"
              : notice
                ? notice
                : ran
                ? `${ran.count > 1 ? `${ran.count} statements · ` : ""}${result?.rows.length.toLocaleString() ?? 0} rows · ${ran.ms.toFixed(1)} ms`
                : "⌘↵ run statement · ⇧⌘↵ run all · ⌘E explain · ⌥⇧F format · ⌘Y history · ⌘S save snippet"}
      </footer>
    </div>
  );
}

/** The Run button triggers the editor's own ⌘↵ path so it sees the cursor. */
function EditorWithRunEvent(props: Parameters<typeof SqlEditor>[0] & { visible: boolean }) {
  const snap = useRef<EditorSnapshot | null>(null);
  const { visible, ...editor } = props;
  useEffect(() => {
    if (!visible) return;
    const at = () => snap.current ?? { text: editor.value, from: 0, to: 0, head: 0 };
    const onRun = () => editor.onRun("statement", at());
    const onExplain = () => editor.onExplain(true, at());
    document.addEventListener("fabio-run", onRun);
    document.addEventListener("fabio-explain", onExplain);
    return () => {
      document.removeEventListener("fabio-run", onRun);
      document.removeEventListener("fabio-explain", onExplain);
    };
  });
  return (
    <SqlEditor
      {...editor}
      onChange={(v) => editor.onChange(v)}
      onRun={(mode, at) => editor.onRun(mode, at)}
      onFormat={editor.onFormat}
      onSelection={(at: EditorSnapshot) => (snap.current = at)}
    />
  );
}

function timeAgo(ms: number) {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return new Date(ms).toLocaleDateString();
}
