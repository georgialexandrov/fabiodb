import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, isNumeric, plural, type ExportFormat, type RelationRef, type ResultColumn, type Rows, type Sort } from "./api";

const ROW_HEIGHT = 26;
const CHAR_WIDTH = 7.4;

type Row = (string | null)[];

type Props = {
  columns: ResultColumn[];
  rowCount: number;
  /** `undefined` while that row's page is still loading. */
  row: (index: number) => Row | undefined;
  /** Rows used to size columns. */
  sample: Rows;
  /** Called with the visible row range whenever it changes. */
  onRange?: (first: number, last: number) => void;
  sort?: Sort | null;
  onSort?: (sort: Sort | null) => void;
  /** Scrolls so `row` is at the top; bump `nonce` to repeat the same jump. */
  scrollTo?: { row: number; nonce: number } | null;
  /** Added to displayed row numbers when the grid shows a window of a larger result. */
  rowOffset?: number;
  /** Cell editing, for tables with a primary key. */
  edit?: GridEdit;
  /** The table these rows come from; enables "Copy as INSERT". */
  table?: RelationRef;
  /** Says what a copy did, in the view's status line. */
  onNotice?: (message: string) => void;
  /** Column filters: whether a column has one, and opening its popover (⌘F too). */
  filtered?: (col: number) => boolean;
  onFilter?: (col: number, anchor: DOMRect) => void;
};

type Cell = { row: number; col: number };

const COPY_FORMATS: { format: ExportFormat; label: string }[] = [
  { format: "csv", label: "CSV" },
  { format: "markdown", label: "Markdown" },
  { format: "json", label: "JSON" },
  { format: "insert", label: "INSERT statements" },
];

export type GridEdit = {
  /** The unsaved value of a cell; `undefined` when it hasn't been edited. */
  pending: (row: number, col: number) => string | null | undefined;
  onEdit: (row: number, col: number, value: string | null) => void;
  /** Why this grid can't be edited; said once when someone tries. */
  blocked: string | null;
  onBlocked: (reason: string) => void;
  nullable: boolean[];
};

/** Virtualized, random-access grid. Only on-screen rows are rendered. */
export function Grid(props: Props) {
  const { columns, rowCount, row, sample, onRange, sort, onSort, scrollTo, rowOffset = 0, edit, table, onNotice } = props;
  const { filtered, onFilter } = props;
  const headerCells = useRef<(HTMLDivElement | null)[]>([]);
  const scroller = useRef<HTMLDivElement>(null);
  // `selected` is the active cell; with `anchor` it spans a rectangle (shift-click, shift-arrows).
  const [selected, setSelected] = useState<Cell | null>(null);
  const [anchor, setAnchor] = useState<Cell | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [editing, setEditing] = useState<{ row: number; col: number; draft: string } | null>(null);
  const [detail, setDetail] = useState<{ column: ResultColumn; value: string | null } | null>(null);

  const widths = useMemo(() => columnWidths(columns, sample), [columns, sample]);
  const totalWidth = widths.reduce((a, b) => a + b, 0) + gutterWidth(rowOffset + rowCount);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });
  const items = virtualizer.getVirtualItems();
  const first = items.length ? items[0].index : 0;
  const last = items.length ? items[items.length - 1].index : 0;

  useEffect(() => {
    onRange?.(first, last);
  }, [first, last, rowCount]);

  useEffect(() => {
    if (scrollTo) virtualizer.scrollToIndex(scrollTo.row, { align: "start" });
  }, [scrollTo]);

  useEffect(() => {
    setSelected(null);
    setAnchor(null);
    setEditing(null);
  }, [columns]);

  const range = selected && {
    top: Math.min(selected.row, (anchor ?? selected).row),
    bottom: Math.max(selected.row, (anchor ?? selected).row),
    left: Math.min(selected.col, (anchor ?? selected).col),
    right: Math.max(selected.col, (anchor ?? selected).col),
  };
  const inRange = (r: number, c: number) =>
    !!range && r >= range.top && r <= range.bottom && c >= range.left && c <= range.right;

  function select(at: Cell, extend: boolean) {
    if (!extend || !selected) setAnchor(at);
    else if (!anchor) setAnchor(selected);
    setSelected(at);
  }

  /** The selected rectangle, loaded rows only, in `format`. */
  async function copy(format: ExportFormat) {
    if (!range) return;
    const cols = columns.slice(range.left, range.right + 1);
    const rows: Row[] = [];
    let missing = 0;
    for (let i = range.top; i <= range.bottom; i++) {
      const r = row(i);
      if (!r) missing++;
      else rows.push(cols.map((_, j) => shown(r, i, range.left + j)));
    }
    const single = rows.length === 1 && cols.length === 1;
    const text =
      single && format === "tsv" ? (rows[0][0] ?? "NULL") : await api.copyRows(cols, rows, format, table ?? null);
    await navigator.clipboard.writeText(text);
    if (!single || format !== "tsv") {
      const skipped = missing ? `; ${plural(missing, "row")} not loaded yet were left out` : "";
      onNotice?.(`Copied ${plural(rows.length, "row")} × ${plural(cols.length, "column")}${skipped}`);
    }
  }

  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && (e.target as HTMLElement).closest(".grid-menu")) return;
      setMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [menu]);

  /** What the cell shows: the unsaved edit if there is one. */
  const shown = (r: Row, rowIndex: number, col: number) => {
    const pending = edit?.pending(rowIndex, col);
    return pending === undefined ? r[col] : pending;
  };

  function startEdit(at: { row: number; col: number }) {
    if (!edit) return;
    if (edit.blocked) return edit.onBlocked(edit.blocked);
    const r = row(at.row);
    if (r) setEditing({ ...at, draft: shown(r, at.row, at.col) ?? "" });
  }

  function commit(value: string | null) {
    if (!editing) return;
    edit?.onEdit(editing.row, editing.col, value);
    setEditing(null);
  }

  function move(dRow: number, dCol: number, extend: boolean) {
    if (!selected) return;
    const next = {
      row: Math.min(Math.max(0, selected.row + dRow), rowCount - 1),
      col: Math.min(Math.max(0, selected.col + dCol), columns.length - 1),
    };
    select(next, extend);
    virtualizer.scrollToIndex(next.row, { align: "auto" });
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      // Only the grid on screen, never while typing somewhere else or with a dialog open.
      if (!selected || editing || !scroller.current?.offsetParent || document.querySelector(".modal-backdrop")) return;
      const target = e.target as HTMLElement;
      if (target.closest("input, textarea, select, .cm-editor")) return;
      const r = row(selected.row);
      if ((e.metaKey || e.ctrlKey) && e.key === "c") {
        if (window.getSelection()?.toString()) return;
        copy("tsv");
      } else if ((e.metaKey || e.ctrlKey) && e.key === "f" && onFilter) {
        const cell = headerCells.current[selected.col];
        if (cell) onFilter(selected.col, cell.getBoundingClientRect());
      } else if (e.metaKey || e.ctrlKey || e.altKey) {
        return;
      } else if (e.key === "ArrowUp") move(-1, 0, e.shiftKey);
      else if (e.key === "ArrowDown") move(1, 0, e.shiftKey);
      else if (e.key === "ArrowLeft") move(0, -1, e.shiftKey);
      else if (e.key === "ArrowRight") move(0, 1, e.shiftKey);
      else if (e.key === "Enter" && edit) startEdit(selected);
      else if (e.key === " " && r) setDetail({ column: columns[selected.col], value: shown(r, selected.row, selected.col) });
      else return;
      e.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function clickHeader(name: string) {
    if (!onSort) return;
    if (sort?.column !== name) onSort({ column: name, descending: false });
    else if (!sort.descending) onSort({ column: name, descending: true });
    else onSort(null);
  }

  const gutter = gutterWidth(rowOffset + rowCount);

  return (
    <div className="grid-wrap">
      <div className="grid" ref={scroller}>
        <div style={{ width: totalWidth, height: virtualizer.getTotalSize() + ROW_HEIGHT, position: "relative" }}>
          <div className="grid-row grid-head" style={{ width: totalWidth }}>
            <div className="grid-cell grid-gutter" style={{ width: gutter }} />
            {columns.map((c, i) => (
              <div
                key={i}
                ref={(el) => {
                  headerCells.current[i] = el;
                }}
                className={`grid-cell ${isNumeric(c.data_type) ? "num" : ""} ${onSort ? "sortable" : ""}`}
                style={{ width: widths[i] }}
                onClick={() => clickHeader(c.name)}
                title={c.data_type}
              >
                <span className="col-name">{c.name}</span>
                {sort?.column === c.name && <span className="sort-mark">{sort.descending ? "↓" : "↑"}</span>}
                <span className="col-type">{c.data_type}</span>
                {onFilter && (
                  <button
                    className={`filter-mark ${filtered?.(i) ? "on" : ""}`}
                    title={filtered?.(i) ? "Filtered — edit conditions" : "Filter this column (⌘F)"}
                    onClick={(e) => {
                      e.stopPropagation();
                      onFilter(i, e.currentTarget.getBoundingClientRect());
                    }}
                  >
                    <svg width="11" height="11" viewBox="0 0 16 16" aria-hidden="true">
                      <path d="M1.5 2h13l-5 6.2V14l-3-1.6V8.2z" fill="currentColor" />
                    </svg>
                  </button>
                )}
              </div>
            ))}
          </div>
          {items.map((item) => {
            const r = row(item.index);
            return (
              <div
                key={item.key}
                className={`grid-row ${r ? "" : "pending"}`}
                style={{ transform: `translateY(${item.start + ROW_HEIGHT}px)`, width: totalWidth }}
              >
                <div className="grid-cell grid-gutter" style={{ width: gutter }}>
                  {(rowOffset + item.index + 1).toLocaleString()}
                </div>
                {columns.map((column, c) => {
                  if (!r) return <div key={c} className="grid-cell" style={{ width: widths[c] }} />;
                  const value = shown(r, item.index, c);
                  const at = { row: item.index, col: c };
                  if (editing?.row === item.index && editing.col === c) {
                    return (
                      <CellEditor
                        key={c}
                        width={widths[c]}
                        draft={editing.draft}
                        nullable={edit?.nullable[c] ?? false}
                        onDraft={(draft) => setEditing({ ...editing, draft })}
                        onCommit={commit}
                        onCancel={() => setEditing(null)}
                      />
                    );
                  }
                  const edited = edit?.pending(item.index, c) !== undefined;
                  return (
                    <div
                      key={c}
                      className={`${cellClass(column, value, selected?.row === item.index && selected.col === c)}${edited ? " edited" : ""}${inRange(item.index, c) ? " in-range" : ""}`}
                      style={{ width: widths[c] }}
                      onClick={(e) => select(at, e.shiftKey)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        if (!inRange(item.index, c)) select(at, false);
                        setMenu({ x: e.clientX, y: e.clientY });
                      }}
                      onDoubleClick={() => (edit ? startEdit(at) : setDetail({ column, value }))}
                      title={edited ? `was ${r[c] ?? "NULL"}` : undefined}
                    >
                      {value === null ? "NULL" : preview(value)}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
      {detail && <ValuePanel {...detail} onClose={() => setDetail(null)} />}
      {menu && (
        <div className="menu grid-menu" style={{ position: "fixed", left: menu.x, top: menu.y, right: "auto" }}>
          <button onClick={() => (setMenu(null), copy("tsv"))}>Copy</button>
          <div className="menu-separator" />
          {COPY_FORMATS.filter((f) => f.format !== "insert" || table).map((f) => (
            <button key={f.format} onClick={() => (setMenu(null), copy(f.format).catch((e) => onNotice?.(String(e))))}>
              Copy as {f.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Enter saves the cell (⌥↵ for a new line), Esc cancels. */
function CellEditor(props: {
  width: number;
  draft: string;
  nullable: boolean;
  onDraft: (draft: string) => void;
  onCommit: (value: string | null) => void;
  onCancel: () => void;
}) {
  const { width, draft, nullable, onDraft, onCommit, onCancel } = props;
  const done = useRef(false);
  const finish = (value: string | null | undefined) => {
    if (done.current) return;
    done.current = true;
    value === undefined ? onCancel() : onCommit(value);
  };
  return (
    <div className="grid-cell editing" style={{ width }}>
      <textarea
        autoFocus
        spellCheck={false}
        value={draft}
        rows={1}
        onFocus={(e) => e.currentTarget.select()}
        onChange={(e) => onDraft(e.target.value)}
        onBlur={() => finish(draft)}
        onKeyDown={(e) => {
          if (e.key === "Escape") finish(undefined);
          else if (e.key === "Enter" && !e.altKey && !e.shiftKey) finish(draft);
          else return;
          e.preventDefault();
          e.stopPropagation();
        }}
      />
      {nullable && (
        <button className="set-null" onMouseDown={(e) => (e.preventDefault(), finish(null))} title="Set to NULL">
          NULL
        </button>
      )}
    </div>
  );
}

function ValuePanel({ column, value, onClose }: { column: ResultColumn; value: string | null; onClose: () => void }) {
  let shown = value ?? "NULL";
  if (value && /json/i.test(column.data_type)) {
    try {
      shown = JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      /* not valid JSON after all; show as-is */
    }
  }
  return (
    <aside className="value-panel">
      <header>
        <span>
          {column.name} <span className="muted">{column.data_type}</span>
        </span>
        <button className="ghost" onClick={onClose}>
          ✕
        </button>
      </header>
      <pre className={value === null ? "null" : ""}>{shown}</pre>
      <footer>
        <button onClick={() => navigator.clipboard.writeText(value ?? "NULL")}>Copy</button>
        {value !== null && <span className="muted">{value.length.toLocaleString()} chars</span>}
      </footer>
    </aside>
  );
}

function cellClass(column: ResultColumn, value: string | null, selected: boolean) {
  const classes = ["grid-cell"];
  if (value === null) classes.push("null");
  else if (isNumeric(column.data_type)) classes.push("num");
  if (selected) classes.push("selected");
  return classes.join(" ");
}

/** First line only, capped — the full value lives in the detail panel. */
function preview(value: string) {
  const line = value.length > 300 ? value.slice(0, 300) : value;
  const nl = line.indexOf("\n");
  return nl === -1 ? line : `${line.slice(0, nl)} ⏎`;
}

/** Wide enough for the largest row number. */
function gutterWidth(rowCount: number) {
  return Math.max(48, rowCount.toLocaleString().length * 8 + 20);
}

function columnWidths(columns: ResultColumn[], rows: Rows) {
  const sample = rows.slice(0, 100);
  return columns.map((c, i) => {
    // Name and type sit side by side in the header; the type is set smaller.
    const header = c.name.length + c.data_type.length * 0.75 + 3;
    const longest = sample.reduce((m, r) => Math.max(m, Math.min((r[i] ?? "NULL").length, 60)), 0);
    return Math.round(Math.min(420, Math.max(64, Math.max(header, longest) * CHAR_WIDTH + 24)));
  });
}
