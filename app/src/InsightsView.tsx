import { useEffect, useState } from "react";
import { api, type Insights } from "./api";

type Props = { connectionId: string; visible: boolean; onOpenQuery: (sql: string) => void };

/** What Postgres knows about itself. Refreshes while on screen. */
export function InsightsView({ connectionId, visible, onOpenQuery }: Props) {
  const [data, setData] = useState<Insights | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    let live = true;
    const load = () =>
      api.insights(connectionId).then(
        (d) => live && (setData(d), setError(null)),
        (e) => live && setError(String(e)),
      );
    load();
    const timer = setInterval(load, 3000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [connectionId, visible]);

  if (error) return <p className="error">{error}</p>;
  if (!data) return <p className="hint">Reading statistics…</p>;

  return (
    <div className="insights">
      <section>
        <h3>Running now</h3>
        {data.activity.length === 0 ? (
          <p className="hint">Nothing else is connected to this database.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>pid</th>
                <th>State</th>
                <th className="num">For</th>
                <th>Waiting on</th>
                <th>Query</th>
              </tr>
            </thead>
            <tbody>
              {data.activity.map((a) => (
                <tr key={a.pid} className={a.blocked_by.length ? "blocked" : ""}>
                  <td>{a.pid}</td>
                  <td>
                    {a.state ?? "–"}
                    {a.application && <span className="muted"> · {a.application}</span>}
                  </td>
                  <td className="num">{a.state === "active" ? duration(a.running_ms) : ""}</td>
                  <td>
                    {a.blocked_by.length > 0 ? <b className="accent">blocked by {a.blocked_by.join(", ")}</b> : (a.waiting_on ?? "")}
                  </td>
                  <td className="sql" title={a.query}>
                    {oneLine(a.query)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <h3>Where the time went</h3>
        {data.top_statements === null ? (
          <p className="hint">
            pg_stat_statements isn't installed. Add it to <code>shared_preload_libraries</code>, restart Postgres, then run{" "}
            <code>CREATE EXTENSION pg_stat_statements</code>.
          </p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Statement</th>
                <th className="num">Calls</th>
                <th className="num">Total</th>
                <th className="num">Mean</th>
                <th className="num">Rows</th>
              </tr>
            </thead>
            <tbody>
              {data.top_statements.slice(0, 25).map((s, i) => (
                <tr key={i} className="clickable" onClick={() => onOpenQuery(s.query)} title="Open in a new query tab">
                  <td className="sql">{oneLine(s.query)}</td>
                  <td className="num">{s.calls.toLocaleString()}</td>
                  <td className="num">{duration(s.total_ms)}</td>
                  <td className="num">{duration(s.mean_ms)}</td>
                  <td className="num">{s.rows.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <h3>Tables read by full scans</h3>
        {data.seq_scan_tables.length === 0 ? (
          <p className="hint">No table has been read by a sequential scan yet.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Table</th>
                <th className="num">Full scans</th>
                <th className="num">Rows read</th>
                <th className="num">Index scans</th>
                <th className="num">Live rows</th>
              </tr>
            </thead>
            <tbody>
              {data.seq_scan_tables.map((t) => (
                <tr key={`${t.schema}.${t.table}`}>
                  <td>
                    <span className="muted">{t.schema}.</span>
                    {t.table}
                  </td>
                  <td className="num">{t.seq_scans.toLocaleString()}</td>
                  <td className="num">{t.seq_rows_read.toLocaleString()}</td>
                  <td className="num">{t.index_scans?.toLocaleString() ?? "–"}</td>
                  <td className="num">{t.live_rows.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <h3>Indexes never used</h3>
        {data.unused_indexes.length === 0 ? (
          <p className="hint">Every index has been used since statistics were last reset.</p>
        ) : (
          <>
            <p className="hint">Never scanned since statistics were last reset. They still cost space and slow down writes.</p>
            <table>
              <tbody>
                {data.unused_indexes.map((i) => (
                  <tr key={`${i.schema}.${i.name}`}>
                    <td>{i.name}</td>
                    <td className="muted">
                      on {i.schema}.{i.table}
                    </td>
                    <td className="num">{size(i.size_bytes)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
      </section>
    </div>
  );
}

const oneLine = (sql: string) => sql.replace(/\s+/g, " ").trim();

function duration(ms: number | null) {
  if (ms == null) return "–";
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)} min`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} s`;
  return `${ms.toFixed(ms >= 10 ? 0 : 2)} ms`;
}

function size(bytes: number) {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} GB`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} MB`;
  return `${Math.round(bytes / 1024)} kB`;
}
