import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Target =
  | { engine: "postgres"; url: string }
  | { engine: "sqlite"; path: string };

type QueryResult = {
  columns: string[];
  rows: (string | null)[][];
  elapsed_ms: number;
};

const DEFAULTS = {
  postgres: "postgres://fabio@localhost:54329/chinook",
  sqlite: "",
};

export default function App() {
  const [engine, setEngine] = useState<Target["engine"]>("postgres");
  const [location, setLocation] = useState(DEFAULTS);
  const [sql, setSql] = useState("select * from artist order by artist_id limit 100");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  async function run() {
    const target: Target =
      engine === "postgres"
        ? { engine, url: location.postgres }
        : { engine, path: location.sqlite };
    setRunning(true);
    setError(null);
    try {
      setResult(await invoke<QueryResult>("run_query", { target, sql }));
    } catch (e) {
      setResult(null);
      setError(String(e));
    } finally {
      setRunning(false);
    }
  }

  return (
    <main>
      <header>
        <span className="brand">Fabio</span>
        <select value={engine} onChange={(e) => setEngine(e.target.value as Target["engine"])}>
          <option value="postgres">Postgres</option>
          <option value="sqlite">SQLite</option>
        </select>
        <input
          className="location"
          spellCheck={false}
          placeholder={engine === "postgres" ? "postgres://…" : "/path/to/file.sqlite"}
          value={location[engine]}
          onChange={(e) => setLocation({ ...location, [engine]: e.target.value })}
        />
      </header>

      <textarea
        className="editor"
        spellCheck={false}
        value={sql}
        onChange={(e) => setSql(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            run();
          }
        }}
      />

      <section className="results">
        {error && <p className="error">{error}</p>}
        {result && (
          <table>
            <thead>
              <tr>
                {result.columns.map((c, i) => (
                  <th key={i}>{c}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {result.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((v, c) =>
                    v === null ? <td key={c} className="null">NULL</td> : <td key={c}>{v}</td>,
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <footer>
        {running
          ? "Running…"
          : result
            ? `${result.rows.length} rows · ${result.elapsed_ms.toFixed(1)} ms`
            : "⌘↵ to run"}
      </footer>
    </main>
  );
}
