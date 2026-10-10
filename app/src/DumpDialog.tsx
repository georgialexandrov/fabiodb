import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { chooseFile } from "./ExportDialog";
import { api, fileName, type DumpOptions, type DumpReport, type JobProgress } from "./api";

type Props = {
  mode: "export" | "import";
  /** The workspace: `connection` or `connection#database`. */
  id: string;
  engine: "postgres" | "sqlite";
  /** The database as people know it: "chinook", or the SQLite file's name. */
  name: string;
  /** Postgres schemas, for picking one; the sidebar's pick comes first. */
  schemas: string[];
  activeSchema: string | null;
  /** After an import: the tables may have changed. */
  onImported: () => void;
  onClose: () => void;
};

type Remembered = Pick<DumpOptions, "content" | "format" | "owners" | "clean">;
const KEY = "fabio.dump";

function remembered(): Remembered {
  const defaults: Remembered = { content: "all", format: "sql", owners: false, clean: false };
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return defaults;
  }
}

const size = (bytes: number) =>
  bytes < 1024 ? `${bytes} B` : bytes < 1024 ** 2 ? `${(bytes / 1024).toFixed(1)} KB` : bytes < 1024 ** 3 ? `${(bytes / 1024 ** 2).toFixed(1)} MB` : `${(bytes / 1024 ** 3).toFixed(2)} GB`;

const seconds = (ms: number) => `${(ms / 1000).toFixed(1)} s`;

/**
 * Export: the whole database to one file (pg_dump for Postgres). Import: a
 * dump run against this database in one transaction. The dialog stays open
 * with the outcome.
 */
export function DumpDialog({ mode, id, engine, name, schemas, activeSchema, onImported, onClose }: Props) {
  const pg = engine === "postgres";
  const [options, setOptions] = useState<Remembered>(remembered);
  const [schema, setSchema] = useState<string>(activeSchema ?? "");
  const [file, setFile] = useState<string | null>(null);
  const [job, setJob] = useState<string | null>(null);
  const [line, setLine] = useState<string | null>(null);
  const [started, setStarted] = useState(0);
  const [, tick] = useState(0);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const jobRef = useRef<string | null>(null);
  jobRef.current = job;

  // A SQLite file copy holds everything.
  const copy = !pg && options.format === "archive";
  const content = copy ? "all" : options.content;

  useEffect(() => {
    const unlisten = listen<JobProgress>("job-progress", (e) => e.payload.job === jobRef.current && setLine(e.payload.line));
    return () => {
      unlisten.then((f) => f());
      if (jobRef.current) api.cancelJob(jobRef.current);
    };
  }, []);

  // The elapsed time moves while a job runs.
  useEffect(() => {
    if (!job) return;
    const timer = window.setInterval(() => tick((n) => n + 1), 500);
    return () => window.clearInterval(timer);
  }, [job]);

  async function run(work: (job: string) => Promise<DumpReport>, describe: (r: DumpReport) => string) {
    const next = `dump-${Date.now().toString(36)}`;
    setJob(next);
    setLine(null);
    setError(null);
    setDone(null);
    setStarted(performance.now());
    try {
      setDone(describe(await work(next)));
    } catch (e) {
      setError(String(e));
    } finally {
      setJob(null);
    }
  }

  async function exportDump() {
    try {
      localStorage.setItem(KEY, JSON.stringify(options));
    } catch {
      // Remembering is a convenience.
    }
    const ext = pg ? (options.format === "archive" ? "dump" : "sql") : copy ? "sqlite" : "sql";
    const base = name.replace(/\.(db|sqlite|sqlite3|db3)$/i, "") + (schema ? `-${schema}` : "");
    const path = await chooseFile(base, ext, ext === "dump" ? "Postgres archive" : ext.toUpperCase());
    if (!path) return;
    const dump: DumpOptions = { ...options, content, schemas: schema ? [schema] : [] };
    await run(
      (job) => api.dumpDatabase(id, job, dump, path),
      (r) => `Saved ${fileName(path)} · ${size(r.bytes)} · ${seconds(r.elapsed_ms)}${r.tool ? ` · ${r.tool}` : ""}`,
    );
  }

  async function pickFile() {
    const extensions = pg ? ["sql", "dump", "backup"] : ["sql", "db", "sqlite", "sqlite3", "db3"];
    const path = await open({ multiple: false, filters: [{ name: "Dump", extensions }, { name: "All files", extensions: ["*"] }] });
    if (typeof path === "string") {
      setFile(path);
      setDone(null);
      setError(null);
    }
  }

  async function importDump() {
    if (!file) return;
    await run(
      (job) => api.restoreDatabase(id, job, file),
      (r) => `Imported ${fileName(file)} · ${size(r.bytes)} · ${seconds(r.elapsed_ms)}${r.tool ? ` · ${r.tool}` : ""}`,
    );
    onImported();
  }

  const busy = job !== null;
  const replaces = !pg && file !== null && !/\.sql$/i.test(file);

  return (
    <div className="modal-backdrop" onMouseDown={() => !busy && onClose()}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()} onKeyDown={(e) => e.key === "Escape" && !busy && onClose()}>
        <h2>
          {mode === "export" ? "Export" : "Import into"} {name}
        </h2>

        {mode === "export" ? (
          <>
            <label>
              Format
              <select autoFocus value={options.format} disabled={busy} onChange={(e) => setOptions({ ...options, format: e.target.value as DumpOptions["format"] })}>
                <option value="sql">SQL file (.sql)</option>
                <option value="archive">{pg ? "Postgres archive (.dump, for pg_restore)" : "Copy of the database file"}</option>
              </select>
            </label>
            <div className="row">
              <label className="grow">
                Content
                <select value={content} disabled={busy || copy} onChange={(e) => setOptions({ ...options, content: e.target.value as DumpOptions["content"] })}>
                  <option value="all">Structure and data</option>
                  <option value="schema">Structure only</option>
                  <option value="data">Data only</option>
                </select>
              </label>
              {pg && schemas.length > 1 && (
                <label className="grow">
                  Schema
                  <select value={schema} disabled={busy} onChange={(e) => setSchema(e.target.value)}>
                    <option value="">Every schema</option>
                    {schemas.map((s) => (
                      <option key={s} value={s}>
                        {s}
                      </option>
                    ))}
                  </select>
                </label>
              )}
            </div>
            {!copy && content !== "data" && (
              <label className="check">
                <input type="checkbox" checked={options.clean} disabled={busy} onChange={(e) => setOptions({ ...options, clean: e.target.checked })} />
                Drop each object before creating it, to restore over an existing copy
              </label>
            )}
            {pg && (
              <label className="check">
                <input type="checkbox" checked={options.owners} disabled={busy} onChange={(e) => setOptions({ ...options, owners: e.target.checked })} />
                Keep owners and privileges (the restoring server needs the same roles)
              </label>
            )}
            {pg && !done && !error && <p className="muted">Uses pg_dump from your Postgres client tools.</p>}
          </>
        ) : (
          <>
            <div className="row dump-file">
              <button autoFocus type="button" onClick={pickFile} disabled={busy}>
                Choose file…
              </button>
              <span className={file ? "ellipsis" : "muted"} title={file ?? undefined}>
                {file ? fileName(file) : pg ? "A .sql file, or an archive from pg_dump" : "A .sql file, or a SQLite database"}
              </span>
            </div>
            {file && !done && !error && (
              <p className="muted">
                {replaces
                  ? `Replaces everything in ${name} with the contents of ${fileName(file)}.`
                  : `Runs in one transaction: if a statement fails, nothing in ${name} changes.`}
                {pg && (/\.sql$/i.test(file) ? " Runs with psql." : " Runs with pg_restore if it's an archive, psql otherwise.")}
              </p>
            )}
          </>
        )}

        {busy && (
          <p className="dump-progress">
            <span className="ellipsis">{line ?? (mode === "export" ? "Starting…" : "Reading the file…")}</span>
            <span className="muted">{seconds(performance.now() - started)}</span>
          </p>
        )}
        {done && <p>{done}</p>}
        {error && <pre className="error dump-error">{error}</pre>}

        <div className="actions">
          <span className="grow" />
          {busy ? (
            <button type="button" onClick={() => job && api.cancelJob(job)}>
              Cancel
            </button>
          ) : (
            <button type="button" onClick={onClose}>
              {done ? "Close" : "Cancel"}
            </button>
          )}
          {mode === "export" ? (
            <button type="button" className="primary" onClick={() => exportDump().catch((e) => setError(String(e)))} disabled={busy}>
              {busy ? "Exporting…" : "Export…"}
            </button>
          ) : (
            <button type="button" className={replaces ? "danger" : "primary"} onClick={importDump} disabled={busy || !file}>
              {busy ? "Importing…" : replaces ? `Replace ${name}` : "Import"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
