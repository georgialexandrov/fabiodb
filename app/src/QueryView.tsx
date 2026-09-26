import { useState } from "react";
import { Grid } from "./Grid";
import { api, type QueryResult } from "./api";

/** Minimal SQL box until the Phase 2 editor replaces it. */
export function QueryView({ connectionId }: { connectionId: string }) {
  const [sql, setSql] = useState("");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  async function run() {
    setRunning(true);
    setError(null);
    try {
      setResult(await api.runQuery(connectionId, sql));
    } catch (e) {
      setResult(null);
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  return (
    <div className="table-view">
      <textarea
        className="editor"
        autoFocus
        spellCheck={false}
        placeholder="select …   (⌘↵ to run)"
        value={sql}
        onChange={(e) => setSql(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            run();
          }
        }}
      />
      {error && <p className="error">{error}</p>}
      {result && (
        <Grid columns={result.columns.map((name) => ({ name, data_type: "" }))} rows={result.rows} />
      )}
      <footer className="status">
        {running ? "Running…" : result ? `${result.rows.length} rows · ${result.elapsed_ms.toFixed(1)} ms` : "⌘↵ to run"}
      </footer>
    </div>
  );
}
