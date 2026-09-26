import type { AuditEntry } from "./api";

type Props = {
  /** This connection's agent statements, newest first. */
  entries: AuditEntry[];
  onOpen: (entry: AuditEntry) => void;
};

/** What agents ran on one connection, as it happens. */
export function AgentView({ entries, onOpen }: Props) {
  if (entries.length === 0) {
    return (
      <div className="agent-empty">
        <p>No agent has queried this connection yet.</p>
        <p className="muted">
          Agents connect through Fabio’s MCP server, read-only. For Claude Code:
          <br />
          <code>claude mcp add fabio -- fabio-mcp</code>
        </p>
      </div>
    );
  }
  return (
    <div className="agent-log">
      {entries.map((e) => (
        <div
          key={e.id}
          className={`agent-entry ${e.error ? "failed" : ""} ${isNote(e) ? "note" : ""}`}
          onClick={() => !isNote(e) && onOpen(e)}
          title={isNote(e) ? undefined : "Open in a query tab and run it again"}
        >
          <span className="agent-time">{new Date(e.at_ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</span>
          <div className="agent-body">
            <div className="agent-sql">{e.sql}</div>
            <div className="agent-meta">{e.error ?? `${outcome(e)} · ${formatMs(e.elapsed_ms)}`}</div>
          </div>
        </div>
      ))}
    </div>
  );
}

/** How the agent ran it: EXPLAIN entries are logged with Fabio's own prefix. */
export function explainMode(sql: string): { mode: "run" | "explain" | "analyze"; sql: string } {
  if (sql.startsWith("EXPLAIN ANALYZE ")) return { mode: "analyze", sql: sql.slice(16) };
  if (sql.startsWith("EXPLAIN ")) return { mode: "explain", sql: sql.slice(8) };
  return { mode: "run", sql };
}

/** Things the agent did that aren't statements, like adding a connection. */
const isNote = (e: AuditEntry) => e.sql.startsWith("-- ");

function outcome(e: AuditEntry) {
  if (isNote(e)) return "connection saved";
  if (explainMode(e.sql).mode !== "run") return "plan";
  return `${(e.rows ?? 0).toLocaleString()} ${e.rows === 1 ? "row" : "rows"}`;
}

export function formatMs(ms: number) {
  return ms < 10 ? `${ms.toFixed(1)} ms` : ms < 10_000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}
