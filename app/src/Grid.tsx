import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { isNumeric, type ResultColumn, type Rows, type Sort } from "./api";

const ROW_HEIGHT = 26;
const CHAR_WIDTH = 7.4;

type Props = {
  columns: ResultColumn[];
  rows: Rows;
  hasMore?: boolean;
  onLoadMore?: () => void;
  sort?: Sort | null;
  onSort?: (sort: Sort | null) => void;
};

/** Virtualized read-only grid. Rows render only while on screen. */
export function Grid({ columns, rows, hasMore, onLoadMore, sort, onSort }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<{ row: number; col: number } | null>(null);
  const [detail, setDetail] = useState<{ column: ResultColumn; value: string | null } | null>(null);

  const widths = useMemo(() => columnWidths(columns, rows), [columns, rows.length > 0]);
  const totalWidth = widths.reduce((a, b) => a + b, 0) + 48;

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });
  const items = virtualizer.getVirtualItems();
  const lastVisible = items.length ? items[items.length - 1].index : 0;

  useEffect(() => {
    if (hasMore && onLoadMore && lastVisible >= rows.length - 50) onLoadMore();
  }, [lastVisible, rows.length, hasMore]);

  useEffect(() => setSelected(null), [columns]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!selected || !(e.metaKey || e.ctrlKey) || e.key !== "c") return;
      if (window.getSelection()?.toString()) return;
      const value = rows[selected.row]?.[selected.col];
      navigator.clipboard.writeText(value ?? "NULL");
      e.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, rows]);

  function clickHeader(name: string) {
    if (!onSort) return;
    if (sort?.column !== name) onSort({ column: name, descending: false });
    else if (!sort.descending) onSort({ column: name, descending: true });
    else onSort(null);
  }

  return (
    <div className="grid-wrap">
      <div className="grid" ref={scroller}>
        <div style={{ width: totalWidth, height: virtualizer.getTotalSize() + ROW_HEIGHT, position: "relative" }}>
          <div className="grid-row grid-head" style={{ width: totalWidth }}>
            <div className="grid-cell grid-gutter" />
            {columns.map((c, i) => (
              <div
                key={i}
                className={`grid-cell ${isNumeric(c.data_type) ? "num" : ""} ${onSort ? "sortable" : ""}`}
                style={{ width: widths[i] }}
                onClick={() => clickHeader(c.name)}
                title={c.data_type}
              >
                <span className="col-name">{c.name}</span>
                {sort?.column === c.name && <span className="sort-mark">{sort.descending ? "↓" : "↑"}</span>}
                <span className="col-type">{c.data_type}</span>
              </div>
            ))}
          </div>
          {items.map((item) => {
            const row = rows[item.index];
            return (
              <div
                key={item.key}
                className="grid-row"
                style={{ transform: `translateY(${item.start + ROW_HEIGHT}px)`, width: totalWidth }}
              >
                <div className="grid-cell grid-gutter">{item.index + 1}</div>
                {row.map((value, c) => (
                  <div
                    key={c}
                    className={cellClass(columns[c], value, selected?.row === item.index && selected.col === c)}
                    style={{ width: widths[c] }}
                    onClick={() => setSelected({ row: item.index, col: c })}
                    onDoubleClick={() => setDetail({ column: columns[c], value })}
                  >
                    {value === null ? "NULL" : preview(value)}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
      {detail && <ValuePanel {...detail} onClose={() => setDetail(null)} />}
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

function columnWidths(columns: ResultColumn[], rows: Rows) {
  const sample = rows.slice(0, 100);
  return columns.map((c, i) => {
    // Name and type sit side by side in the header; the type is set smaller.
    const header = c.name.length + c.data_type.length * 0.75 + 3;
    const longest = sample.reduce((m, r) => Math.max(m, Math.min((r[i] ?? "NULL").length, 60)), 0);
    return Math.round(Math.min(420, Math.max(64, Math.max(header, longest) * CHAR_WIDTH + 24)));
  });
}
