import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { fileName, plural, type ExportFormat } from "./api";

const EXTENSIONS: Record<ExportFormat, string> = {
  csv: "csv",
  spreadsheet_csv: "csv",
  tsv: "tsv",
  json: "json",
  markdown: "md",
  insert: "sql",
  ascii: "txt",
};

const FORMATS: { format: ExportFormat; label: string }[] = [
  { format: "csv", label: "CSV" },
  { format: "spreadsheet_csv", label: "CSV for spreadsheets" },
  { format: "json", label: "JSON" },
  { format: "insert", label: "INSERT statements" },
  { format: "ascii", label: "ASCII table" },
  { format: "markdown", label: "Markdown" },
];

export const formatLabel = (format: ExportFormat) => FORMATS.find((f) => f.format === format)?.label ?? format;

/** The clipboard takes this many rows at most; more goes to a file. */
const COPY_LIMIT = 100_000;

/** Asks where to save; `null` when the dialog is cancelled. */
export async function chooseFile(baseName: string, ext: string, kind = ext.toUpperCase()): Promise<string | null> {
  const path = await save({ defaultPath: `${baseName}.${ext}`, filters: [{ name: kind, extensions: [ext] }] });
  return path ?? null;
}

export type ExportSource = {
  /** What's exported, as the title says it: "track", "the result", "3 selected rows". */
  title: string;
  /** The file name offered, without extension. */
  baseName: string;
  /** How many rows there are; `null` while unknown. */
  total: number | null;
  /** INSERT statements need a table. */
  hasTable: boolean;
  /** One line about which rows these are (filters, order, a capped result). */
  note?: string;
  /** Writes the rows (the first `limit`, if given); returns how many. */
  save: (format: ExportFormat, limit: number | null, path: string) => Promise<number>;
  copy: (format: ExportFormat, limit: number | null) => Promise<{ text: string; rows: number }>;
};

type Remembered = { format: ExportFormat; limit: number; first: boolean };
const KEY = "fabio.export";

function remembered(): Remembered {
  try {
    return { format: "csv", limit: 100, first: false, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return { format: "csv", limit: 100, first: false };
  }
}

/** Format, which rows, then Copy or Save. The format and row count are kept for next time. */
export function ExportDialog({ source, onDone, onClose }: { source: ExportSource; onDone: (message: string) => void; onClose: () => void }) {
  const initial = remembered();
  const [format, setFormat] = useState<ExportFormat>(initial.format === "insert" && !source.hasTable ? "csv" : initial.format);
  const [first, setFirst] = useState(initial.first);
  const [limitText, setLimitText] = useState(String(initial.limit));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const limitValue = Math.floor(Number(limitText));
  const limitValid = limitValue >= 1 && Number.isFinite(limitValue);
  const limit = first ? limitValue : null;
  const rows = limit ?? source.total;
  const tooBigToCopy = rows === null ? !first : rows > COPY_LIMIT;

  function keep() {
    try {
      localStorage.setItem(KEY, JSON.stringify({ format, first, limit: limitValid ? limitValue : 100 }));
    } catch {
      // Remembering is a convenience.
    }
  }

  async function act(run: () => Promise<string | null>) {
    setBusy(true);
    setError(null);
    keep();
    try {
      const message = await run();
      if (message) {
        onDone(message);
        onClose();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const saveFile = () =>
    act(async () => {
      const path = await chooseFile(source.baseName, EXTENSIONS[format]);
      if (!path) return null;
      const started = performance.now();
      const written = await source.save(format, limit, path);
      return `Saved ${plural(written, "row")} to ${fileName(path)} · ${((performance.now() - started) / 1000).toFixed(1)} s`;
    });

  const copy = () =>
    act(async () => {
      const { text, rows } = await source.copy(format, limit);
      await navigator.clipboard.writeText(text);
      return `Copied ${plural(rows, "row")} as ${formatLabel(format)}`;
    });

  return (
    <div className="modal-backdrop" onMouseDown={() => !busy && onClose()}>
      <form
        className="modal"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && !busy && onClose()}
        onSubmit={(e) => {
          e.preventDefault();
          saveFile();
        }}
      >
        <h2>Export {source.title}</h2>
        <label>
          Format
          <select autoFocus value={format} onChange={(e) => setFormat(e.target.value as ExportFormat)}>
            {FORMATS.filter((f) => f.format !== "insert" || source.hasTable).map((f) => (
              <option key={f.format} value={f.format}>
                {f.label}
              </option>
            ))}
          </select>
        </label>
        <div className="export-rows">
          <label className="check">
            <input type="radio" name="rows" checked={!first} onChange={() => setFirst(false)} />
            {source.total === null ? "All rows" : `All ${plural(source.total, "row")}`}
          </label>
          <label className="check">
            <input type="radio" name="rows" checked={first} onChange={() => setFirst(true)} />
            First
            <input
              className="export-limit"
              type="number"
              min={1}
              value={limitText}
              onFocus={() => setFirst(true)}
              onChange={(e) => setLimitText(e.target.value)}
            />
            rows
          </label>
        </div>
        {source.note && <p className="muted">{source.note}</p>}
        {format === "ascii" && !first && (source.total ?? Infinity) > COPY_LIMIT && (
          <p className="muted">An ASCII table is laid out once every row is in, so it's held in memory until then.</p>
        )}
        {error && <p className="error">{error}</p>}
        <div className="actions">
          <span className="grow" />
          <button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            type="button"
            onClick={copy}
            disabled={busy || (first && !limitValid) || tooBigToCopy}
            title={tooBigToCopy ? `The clipboard takes up to ${COPY_LIMIT.toLocaleString()} rows; save to a file instead.` : undefined}
          >
            Copy
          </button>
          <button type="submit" className="primary" disabled={busy || (first && !limitValid)}>
            {busy ? "Exporting…" : "Save…"}
          </button>
        </div>
      </form>
    </div>
  );
}

/** First `limit` rows of rows already in memory. */
export const firstRows = <T,>(rows: T[], limit: number | null) => (limit === null ? rows : rows.slice(0, limit));
