import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ResizeHandle } from "./resize";
import { toggleRowPane, useRowPane } from "./rowPane";
import { clampColumnWidth, fitColumnWidth, preview } from "./columnWidth";
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
  /** Foreign keys: an arrow in their cells follows them; hovering it shows the row they point at. */
  link?: GridLink;
  /** Selects this cell and starts editing it (a new row's first cell); bump `nonce` to repeat. */
  editAt?: { row: number; col: number; nonce: number } | null;
};

export type GridLink = {
  has: (col: number) => boolean;
  onFollow: (row: number, col: number) => void;
  peek: (row: number, col: number) => Promise<LinkedRow>;
};

/** The row a foreign key points at; `row` is null when there's no such row. */
export type LinkedRow = { title: string; columns: string[]; row: (string | null)[] | null };

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
  /** Rows marked for deletion (struck through until saved). */
  deleted: (row: number) => boolean;
  /** Marks the rows, or unmarks them if all are marked already. */
  onDeleteRows: (rows: number[]) => void;
  /** Rows added here and not saved yet; their untouched cells get the column default. */
  isNew?: (row: number) => boolean;
  isDefault?: (row: number, col: number) => boolean;
};

/** Virtualized, random-access grid. Only on-screen rows are rendered. */
export function Grid(props: Props) {
  const { columns, rowCount, row, sample, onRange, sort, onSort, scrollTo, rowOffset = 0, edit, table, onNotice } = props;
  const { filtered, onFilter, link, editAt } = props;
  const headerCells = useRef<(HTMLDivElement | null)[]>([]);
  const scroller = useRef<HTMLDivElement>(null);
  // `selected` is the active cell; with `anchor` it spans a rectangle (shift-click, shift-arrows).
  const [selected, setSelected] = useState<Cell | null>(null);
  const [anchor, setAnchor] = useState<Cell | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  // The selected row, one field per line, beside the grid (⌘I). One setting for all grids.
  const rowPane = useRowPane();
  const [editing, setEditing] = useState<{ row: number; col: number; draft: string } | null>(null);
  const [detail, setDetail] = useState<{ column: ResultColumn; value: string | null } | null>(null);
  // The row behind a foreign-key arrow, shown while the pointer rests on it.
  const [peek, setPeek] = useState<{ at: DOMRect; linked: LinkedRow | null } | null>(null);
  const peekTimer = useRef<number | undefined>(undefined);
  const peekFor = useRef(0);

  function startPeek(target: HTMLElement, row: number, col: number) {
    window.clearTimeout(peekTimer.current);
    const ticket = ++peekFor.current;
    peekTimer.current = window.setTimeout(() => {
      const at = target.getBoundingClientRect();
      setPeek({ at, linked: null });
      link?.peek(row, col).then(
        (linked) => ticket === peekFor.current && setPeek({ at, linked }),
        () => ticket === peekFor.current && setPeek(null),
      );
    }, 250);
  }
  function endPeek() {
    window.clearTimeout(peekTimer.current);
    peekFor.current++;
    setPeek(null);
  }

  const autoWidths = useMemo(() => columnWidths(columns, sample), [columns, sample]);
  // Widths set by hand (dragged, or fitted by double-click), kept while the columns stay the same.
  const columnsKey = columns.map((c) => c.name).join("\u0000");
  const [manual, setManual] = useState<{ key: string; widths: Record<number, number> }>({ key: "", widths: {} });
  const widths = manual.key === columnsKey ? autoWidths.map((w, i) => manual.widths[i] ?? w) : autoWidths;
  // A drag that ends over the header would otherwise click it and sort.
  const justResized = useRef(false);

  function setWidth(col: number, width: number) {
    setManual((m) => ({ key: columnsKey, widths: { ...(m.key === columnsKey ? m.widths : {}), [col]: width } }));
  }

  function startResize(e: React.PointerEvent<HTMLDivElement>, col: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const [x0, w0] = [e.clientX, widths[col]];
    document.body.classList.add("resizing");
    const move = (m: PointerEvent) => setWidth(col, clampColumnWidth(w0 + m.clientX - x0));
    const up = () => {
      document.body.classList.remove("resizing");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      justResized.current = true;
      window.setTimeout(() => (justResized.current = false));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  /** Double-click on the edge: as wide as the header and the loaded values need. */
  function fitColumn(col: number) {
    const head = headerCells.current[col];
    if (!head || !scroller.current) return;
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) return;
    const style = getComputedStyle(scroller.current);
    ctx.font = `${style.fontSize} ${style.fontFamily}`;
    // The header's parts at their natural width, plus its padding and the gaps between them.
    const parts = Array.from(head.children).filter((el) => !el.classList.contains("col-resize")) as HTMLElement[];
    const header = parts.reduce((w, el) => w + el.scrollWidth, 0) + (parts.length - 1) * 6 + 21;
    const values: (string | null)[] = [];
    const seen = new Set<number>();
    const add = (i: number) => {
      if (seen.has(i) || i >= rowCount) return;
      seen.add(i);
      const r = row(i);
      if (r) values.push(shown(r, i, col));
    };
    for (let i = 0; i < Math.min(sample.length, 1000); i++) add(i);
    for (const item of items) add(item.index);
    const arrow = link?.has(col) ? 18 : 0;
    setWidth(col, fitColumnWidth(header, values, (t) => ctx.measureText(t).width + arrow));
  }

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

  const rangeRows = () => (range ? Array.from({ length: range.bottom - range.top + 1 }, (_, i) => range.top + i) : []);

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

  // View ▸ Row Details from the menu bar: the grid on screen answers.
  useEffect(() => {
    const onToggle = () => scroller.current?.offsetParent && toggleRowPane();
    document.addEventListener("fabio-row-pane", onToggle);
    return () => document.removeEventListener("fabio-row-pane", onToggle);
  });

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

  /** Tab / ⇧Tab: keep the value, edit the next cell in the row. */
  function commitAndStep(value: string | null, back: boolean) {
    if (!editing) return;
    const { row: r, col } = editing;
    commit(value);
    const next = col + (back ? -1 : 1);
    if (next < 0 || next >= columns.length) return;
    const at = { row: r, col: next };
    select(at, false);
    const values = row(r);
    if (values) setEditing({ ...at, draft: shown(values, r, next) ?? "" });
  }

  useEffect(() => {
    if (!editAt) return;
    const at = { row: editAt.row, col: editAt.col };
    virtualizer.scrollToIndex(at.row, { align: "auto" });
    select(at, false);
    startEdit(at);
  }, [editAt?.nonce]);

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
      } else if ((e.metaKey || e.ctrlKey) && e.key === "Backspace" && edit && range) {
        if (edit.blocked) edit.onBlocked(edit.blocked);
        else edit.onDeleteRows(rangeRows());
      } else if ((e.metaKey || e.ctrlKey) && e.key === "i") {
        toggleRowPane();
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
    if (!onSort || justResized.current) return;
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
            <div className="grid-cell grid-gutter" style={{ width: gutter }}>
            </div>
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
                <div
                  className="col-resize"
                  onPointerDown={(e) => startResize(e, i)}
                  onClick={(e) => e.stopPropagation()}
                  onDoubleClick={(e) => (e.stopPropagation(), fitColumn(i))}
                  title="Drag to resize; double-click to fit"
                />
              </div>
            ))}
          </div>
          {items.map((item) => {
            const r = row(item.index);
            return (
              <div
                key={item.key}
                className={`grid-row ${r ? "" : "pending"} ${r && edit?.deleted(item.index) ? "deleted" : ""}`}
                style={{ transform: `translateY(${item.start + ROW_HEIGHT}px)`, width: totalWidth }}
              >
                <div className="grid-cell grid-gutter" style={{ width: gutter }}>
                  {edit?.isNew?.(item.index) ? <span className="accent">+</span> : (rowOffset + item.index + 1).toLocaleString()}
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
                        onStep={commitAndStep}
                        onCancel={() => setEditing(null)}
                      />
                    );
                  }
                  const edited = edit?.pending(item.index, c) !== undefined;
                  const linked = value !== null && !edited && !!link?.has(c);
                  const byDefault = !edited && !!edit?.isDefault?.(item.index, c);
                  return (
                    <div
                      key={c}
                      className={`${cellClass(column, value, selected?.row === item.index && selected.col === c)}${edited ? " edited" : ""}${inRange(item.index, c) ? " in-range" : ""}${linked ? " linked" : ""}${byDefault ? " null" : ""}`}
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
                      {byDefault ? (
                        "DEFAULT"
                      ) : linked ? (
                        <>
                          <span className="cell-text">{preview(value)}</span>
                          <button
                            className="fk-link"
                            aria-label="Open the row this points to"
                            onMouseDown={(e) => e.stopPropagation()}
                            onClick={(e) => {
                              e.stopPropagation();
                              endPeek();
                              link!.onFollow(item.index, c);
                            }}
                            onDoubleClick={(e) => e.stopPropagation()}
                            onPointerEnter={(e) => startPeek(e.currentTarget, item.index, c)}
                            onPointerLeave={endPeek}
                          >
                            →
                          </button>
                        </>
                      ) : value === null ? (
                        "NULL"
                      ) : (
                        preview(value)
                      )}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
      {rowPane && !detail && (
        <RowPane
          columns={columns}
          rowNumber={selected ? rowOffset + selected.row + 1 : null}
          values={selected ? (row(selected.row) ?? null)?.map((_, c) => shown(row(selected.row)!, selected.row, c)) ?? null : null}
          edited={(c) => !!selected && edit?.pending(selected.row, c) !== undefined}
          edit={
            edit && !edit.blocked && selected
              ? { onEdit: (c, v) => edit.onEdit(selected.row, c, v), nullable: edit.nullable }
              : null
          }
          onClose={toggleRowPane}
        />
      )}
      {detail && <ValuePanel {...detail} onClose={() => setDetail(null)} />}
      {peek && <LinkPeek at={peek.at} linked={peek.linked} />}
      {menu && (
        <div className="menu grid-menu" style={{ position: "fixed", left: menu.x, top: menu.y, right: "auto" }}>
          <button onClick={() => (setMenu(null), copy("tsv"))}>Copy</button>
          <div className="menu-separator" />
          {edit && !edit.blocked && (
            <>
              <button onClick={() => (setMenu(null), edit.onDeleteRows(rangeRows()))}>
                {rangeRows().every((i) => edit.deleted(i)) ? "Keep" : "Delete"}{" "}
                {rangeRows().length === 1 ? "row" : `${rangeRows().length} rows`} <kbd>⌘⌫</kbd>
              </button>
              <div className="menu-separator" />
            </>
          )}
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

/** The selected row as a form. Fields edit through the same pending edits as cells. */
function RowPane(props: {
  columns: ResultColumn[];
  rowNumber: number | null;
  values: (string | null)[] | null;
  edited: (col: number) => boolean;
  edit: { onEdit: (col: number, value: string | null) => void; nullable: boolean[] } | null;
  onClose: () => void;
}) {
  const { columns, rowNumber, values, edited, edit, onClose } = props;
  return (
    <aside className="row-pane">
      <ResizeHandle name="panel" />
      <header>
        <span>{rowNumber === null ? "No row selected" : `Row ${rowNumber.toLocaleString()}`}</span>
        <button className="ghost" onClick={onClose} title="Hide (⌘I)">
          ✕
        </button>
      </header>
      {values && (
        <div className="row-pane-fields">
          {columns.map((c, i) => (
            <label key={`${rowNumber}-${i}`} className={`row-field ${edited(i) ? "edited" : ""}`}>
              <span className="row-field-name">
                {c.name} <span className="muted">{c.data_type}</span>
              </span>
              {edit ? (
                <RowField value={values[i]} nullable={edit.nullable[i]} onCommit={(v) => edit.onEdit(i, v)} />
              ) : (
                <div className={`row-field-value ${values[i] === null ? "null" : ""}`}>{values[i] ?? "NULL"}</div>
              )}
            </label>
          ))}
        </div>
      )}
    </aside>
  );
}

/** Commits on blur or ⌘↵; Esc puts back what was there. */
function RowField({ value, nullable, onCommit }: { value: string | null; nullable: boolean; onCommit: (v: string | null) => void }) {
  const [draft, setDraft] = useState(value ?? "");
  useEffect(() => setDraft(value ?? ""), [value]);
  const box = useRef<HTMLTextAreaElement>(null);
  // As tall as its text (wrapped lines too), up to a limit.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "0";
    el.style.height = `${Math.min(240, el.scrollHeight + 2)}px`;
  }, [draft]);
  const commit = () => {
    if (draft !== (value ?? "") || (value === null && draft !== "")) onCommit(draft);
  };
  return (
    <div className="row-field-edit">
      <textarea
        spellCheck={false}
        value={draft}
        ref={box}
        placeholder={value === null ? "NULL" : ""}
        rows={1}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Escape") setDraft(value ?? "");
          else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) commit();
          else return;
          e.preventDefault();
          e.stopPropagation();
        }}
      />
      {nullable && value !== null && (
        <button className="ghost set-null-field" onMouseDown={(e) => (e.preventDefault(), onCommit(null))} title="Set to NULL">
          NULL
        </button>
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
  onStep: (value: string | null, back: boolean) => void;
  onCancel: () => void;
}) {
  const { width, draft, nullable, onDraft, onCommit, onStep, onCancel } = props;
  const done = useRef(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const finish = (value: string | null | undefined) => {
    if (done.current) return;
    done.current = true;
    value === undefined ? onCancel() : onCommit(value);
  };
  // Grows with what's typed: wider up to a limit, then taller, over the cells around it.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const longest = draft.split("\n").reduce((m, line) => Math.max(m, line.length), 0);
    el.style.width = `${Math.min(560, Math.max(width, longest * CHAR_WIDTH + 30))}px`;
    el.style.height = "0";
    el.style.height = `${Math.min(320, Math.max(ROW_HEIGHT, el.scrollHeight))}px`;
  }, [draft, width]);
  return (
    <div className="grid-cell editing" style={{ width }}>
      <div className="cell-float">
        <textarea
          ref={box}
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
            else if (e.key === "Tab" && !done.current) {
              done.current = true;
              onStep(draft, e.shiftKey);
            } else return;
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
      <ResizeHandle name="panel" />
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

/** The referenced row, beside the arrow: one field per line. */
function LinkPeek({ at, linked }: { at: DOMRect; linked: LinkedRow | null }) {
  const width = 320;
  const left = Math.min(at.right + 8, window.innerWidth - width - 8);
  const style = { left, top: Math.min(at.top - 6, window.innerHeight - 320), width };
  return (
    <div className="link-peek" style={style}>
      {!linked ? (
        <p className="muted">Looking…</p>
      ) : (
        <>
          <header className="muted ellipsis">{linked.title}</header>
          {linked.row === null ? (
            <p className="muted">No such row.</p>
          ) : (
            <dl>
              {linked.columns.map((name, i) => (
                <div key={i}>
                  <dt className="ellipsis">{name}</dt>
                  <dd className={`ellipsis ${linked.row![i] === null ? "null" : ""}`}>{linked.row![i] === null ? "NULL" : preview(linked.row![i]!)}</dd>
                </div>
              ))}
            </dl>
          )}
        </>
      )}
    </div>
  );
}

function cellClass(column: ResultColumn, value: string | null, selected: boolean) {
  const classes = ["grid-cell"];
  if (value === null) classes.push("null");
  else if (isNumeric(column.data_type)) classes.push("num");
  if (selected) classes.push("selected");
  return classes.join(" ");
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
