import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Grid } from "./Grid";
import { api, type AuditEntry, type CompletionTable, type Plan, type QueryError, type QueryResult } from "./api";
import { PlanView } from "./PlanView";
import { splitStatements, statementAt, type Statement } from "./statements";
import type { EditorSnapshot } from "./SqlEditor";

const SqlEditor = lazy(() => import("./SqlEditor"));

type Props = {
  connectionId: string;
  engine: "postgres" | "sqlite";
  schema: CompletionTable[];
  sql: string;
  onSqlChange: (sql: string) => void;
  visible: boolean;
};

type Failure = { message: string; statement?: Statement; position?: number | null };

export function QueryTab({ connectionId, engine, schema, sql, onSqlChange, visible }: Props) {
  const [session, setSession] = useState<string | null>(null);
  const [writable, setWritable] = useState(false);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [ran, setRan] = useState<{ count: number; ms: number } | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const [plan, setPlan] = useState<{ sql: string; plan: Plan; previous: Plan | null } | null>(null);
  const [view, setView] = useState<"results" | "plan">("results");
  const [history, setHistory] = useState<AuditEntry[]>([]);
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

  const refreshHistory = () => api.history(connectionId, 200).then(setHistory, () => {});
  useEffect(() => {
    if (showHistory) refreshHistory();
  }, [showHistory]);

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
    setView("results");
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
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, running, session]);

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
        <button className={writable ? "danger on" : "ghost"} onClick={toggleWrite} disabled={!session} title="Toggle write mode for this tab">
          {writable ? "Write" : "Read-only"}
        </button>
        <button className={`ghost ${showHistory ? "on" : ""}`} onClick={() => setShowHistory(!showHistory)} title="History (⌘Y)">
          History
        </button>
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
          {plan && result && (
            <div className="segmented result-switch">
              <button className={view === "results" ? "on" : ""} onClick={() => setView("results")}>
                Results
              </button>
              <button className={view === "plan" ? "on" : ""} onClick={() => setView("plan")}>
                Plan
              </button>
            </div>
          )}
          {view === "plan" && plan ? (
            <PlanView plan={plan.plan} previous={plan.previous} engine={engine} />
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
            />
          ) : (
            <div className="grid-wrap" />
          )}
            </>
          )}
        </div>

        {showHistory && (
          <aside className="history">
            {history.length === 0 && <p className="hint">Statements you run here show up in this list.</p>}
            {history.map((h) => (
              <div key={h.id} className={`history-item ${h.error ? "failed" : ""}`} onClick={() => onSqlChange(h.sql)} title={h.error ?? h.sql}>
                <div className="history-sql">{h.sql}</div>
                <div className="history-meta">
                  {timeAgo(h.at_ms)} · {h.error ? "failed" : `${h.rows ?? 0} rows`} · {h.elapsed_ms.toFixed(0)} ms
                  {h.source === "agent" && " · agent"}
                </div>
              </div>
            ))}
          </aside>
        )}
      </div>

      <footer className="status">
        {!session
          ? "Opening a session…"
          : running
            ? "Running… Esc to cancel"
            : failure
              ? "Failed"
              : ran
                ? `${ran.count > 1 ? `${ran.count} statements · ` : ""}${result?.rows.length.toLocaleString() ?? 0} rows · ${ran.ms.toFixed(1)} ms`
                : "⌘↵ run statement · ⇧⌘↵ run all · ⌘E explain · ⌥⇧F format · ⌘Y history"}
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
