import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import type { ExportFormat } from "./api";

export type ExportItem = { label: string; run: () => Promise<string | null> } | "separator";

const EXTENSIONS: Record<ExportFormat, string> = { csv: "csv", json: "json", markdown: "md", insert: "sql" };

/** Asks where to save; `null` when the dialog is cancelled. */
export async function chooseFile(baseName: string, format: ExportFormat): Promise<string | null> {
  const ext = EXTENSIONS[format];
  const path = await save({ defaultPath: `${baseName}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  return path ?? null;
}

type Props = {
  items: ExportItem[];
  /** One line under the items, e.g. that only the shown rows are included. */
  note?: string;
  disabled?: boolean;
  /** Shows what the last action did (or why it failed). */
  onDone: (message: string) => void;
};

/** "Export" button with a small menu. Each item returns a message for the status line. */
export function ExportMenu({ items, note, disabled, onDone }: Props) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !root.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [open]);

  async function pick(run: () => Promise<string | null>) {
    setOpen(false);
    setBusy(true);
    try {
      const message = await run();
      if (message) onDone(message);
    } catch (e) {
      onDone(`Export failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="menu-anchor" ref={root}>
      <button className="ghost" disabled={disabled || busy} onClick={() => setOpen(!open)}>
        {busy ? "Exporting…" : "Export"}
      </button>
      {open && (
        <div className="menu" role="menu">
          {items.map((item, i) =>
            item === "separator" ? (
              <div key={i} className="menu-separator" />
            ) : (
              <button key={i} role="menuitem" onClick={() => pick(item.run)}>
                {item.label}
              </button>
            ),
          )}
          {note && <p className="menu-note">{note}</p>}
        </div>
      )}
    </div>
  );
}
