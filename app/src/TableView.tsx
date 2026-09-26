import { useCallback, useEffect, useRef, useState } from "react";
import { chooseFile, ExportMenu } from "./ExportMenu";
import { Grid } from "./Grid";
import { Structure } from "./Structure";
import {
  api,
  fileName,
  plural,
  type Count,
  type ExportFormat,
  type Filter,
  type FilterOp,
  type Page,
  type Relation,
  type RelationRef,
  type Rows,
  type Sort,
  type TableInfo,
} from "./api";

const PAGE_SIZE = 100;
// WebKit can't lay out elements much taller than ~33M px (1.3M rows at 26 px).
// Bigger results scroll within a window of this many rows that moves on jumps.
const WINDOW_ROWS = 500_000;

const OPS: { op: FilterOp; label: string }[] = [
  { op: "contains", label: "contains" },
  { op: "eq", label: "=" },
  { op: "ne", label: "≠" },
  { op: "lt", label: "<" },
  { op: "le", label: "≤" },
  { op: "gt", label: ">" },
  { op: "ge", label: "≥" },
  { op: "is_null", label: "is null" },
  { op: "is_not_null", label: "is not null" },
];
const needsValue = (op: FilterOp) => op !== "is_null" && op !== "is_not_null";

type Props = { connectionId: string; relation: Relation; onOpen: (r: RelationRef) => void };

export function TableView({ connectionId, relation, onOpen }: Props) {
  const [tab, setTab] = useState<"data" | "structure">("data");
  const [info, setInfo] = useState<TableInfo | null>(null);
  const [sort, setSort] = useState<Sort | null>(null);
  const [draft, setDraft] = useState<Filter[]>([]);
  const [filters, setFilters] = useState<Filter[]>([]);

  // Pages load independently, so any page can be reached by scrolling or jumping.
  const [pages, setPages] = useState<Record<number, Rows>>({});
  const [meta, setMeta] = useState<Omit<Page, "rows" | "has_more"> | null>(null);
  const [lastPage, setLastPage] = useState<number | null>(null);
  const [count, setCount] = useState<Count | "counting" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [firstVisible, setFirstVisible] = useState(0);
  const [jump, setJump] = useState<{ row: number; nonce: number } | null>(null);
  const [base, setBase] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);
  const generation = useRef(0);
  const requested = useRef(new Set<number>());

  useEffect(() => {
    setSort(null);
    setDraft([]);
    setFilters([]);
    setInfo(null);
    api.describe(connectionId, relation).then(setInfo, (e) => setError(String(e)));
  }, [connectionId, relation.schema, relation.name]);

  const loadPage = useCallback(
    async (n: number, gen: number) => {
      requested.current.add(n);
      try {
        const p = await api.page(connectionId, { relation, sort, filters, offset: n * PAGE_SIZE, limit: PAGE_SIZE });
        if (gen !== generation.current) return; // sort/filter/table changed meanwhile
        setPages((prev) => ({ ...prev, [n]: p.rows }));
        setMeta({ columns: p.columns, elapsed_ms: p.elapsed_ms, sql: p.sql });
        if (!p.has_more) setLastPage((prev) => (prev === null ? n : Math.min(prev, n)));
      } catch (e) {
        if (gen !== generation.current) return;
        requested.current.delete(n);
        setError(String(e));
      }
    },
    [connectionId, relation.schema, relation.name, sort, filters],
  );

  // Reset whenever what we're looking at changes.
  useEffect(() => {
    const gen = ++generation.current;
    requested.current = new Set();
    setPages({});
    setLastPage(null);
    setError(null);
    setCount("counting");
    setBase(0);
    setJump({ row: 0, nonce: gen });
    // First page before the count, so rows appear before the (slower) total.
    loadPage(0, gen).then(() =>
      api.count(connectionId, relation, filters).then(
        (c) => gen === generation.current && setCount(c),
        () => gen === generation.current && setCount(null),
      ),
    );
  }, [loadPage]);

  const loadedEnd = Object.keys(pages).reduce((m, k) => Math.max(m, (Number(k) + 1) * PAGE_SIZE), 0);
  const exactTotal =
    count !== "counting" && count?.exact && count.rows != null
      ? count.rows
      : lastPage !== null && pages[lastPage]
        ? lastPage * PAGE_SIZE + pages[lastPage].length
        : null;
  const estimate = count !== "counting" && count && !count.exact ? count.rows : null;
  // Unknown total: size to the estimate, or keep one page of runway past what's loaded.
  const rowCount = exactTotal ?? Math.max(estimate ?? 0, loadedEnd + PAGE_SIZE);
  const pageCount = Math.max(1, Math.ceil(rowCount / PAGE_SIZE));
  const currentPage = Math.min(pageCount, Math.floor(firstVisible / PAGE_SIZE) + 1);

  function onRange(windowFirst: number, windowLast: number) {
    const [first, last] = [base + windowFirst, base + windowLast];
    setFirstVisible(first);
    const gen = generation.current;
    for (let n = Math.floor(first / PAGE_SIZE); n <= Math.floor(last / PAGE_SIZE); n++) {
      if (lastPage !== null && n > lastPage) break;
      if (!requested.current.has(n)) loadPage(n, gen);
    }
  }

  function goTo(page: number) {
    const n = Math.min(Math.max(1, page), pageCount);
    const target = (n - 1) * PAGE_SIZE;
    let nextBase = base;
    if (target < base || target >= base + WINDOW_ROWS) {
      // Recentre the window on the target, page-aligned.
      const centred = Math.max(0, Math.min(target - WINDOW_ROWS / 2, rowCount - WINDOW_ROWS));
      nextBase = Math.floor(centred / PAGE_SIZE) * PAGE_SIZE;
      setBase(nextBase);
    }
    setJump({ row: target - nextBase, nonce: Date.now() });
  }

  function applyFilters(next = draft) {
    setFilters(next.filter((f) => !needsValue(f.op) || (f.value ?? "") !== ""));
  }

  function updateDraft(i: number, patch: Partial<Filter>, apply = false) {
    const next = draft.map((f, j) => (j === i ? { ...f, ...patch } : f));
    setDraft(next);
    if (apply) applyFilters(next);
  }

  const columns = info?.columns ?? [];

  /** Every row under the current sort and filters, streamed to a file by the core. */
  async function exportTable(format: ExportFormat) {
    const path = await chooseFile(relation.name, format);
    if (!path) return null;
    const started = performance.now();
    const rows = await api.exportTable(connectionId, relation, sort, filters, format, path);
    return `Saved ${plural(rows, "row")} to ${fileName(path)} · ${((performance.now() - started) / 1000).toFixed(1)} s`;
  }

  return (
    <div className="table-view">
      <div className="toolbar" data-tauri-drag-region>
        <div className="title">
          <span className="muted">{relation.schema}.</span>
          {relation.name}
        </div>
        <div className="segmented">
          <button className={tab === "data" ? "on" : ""} onClick={() => setTab("data")}>
            Data
          </button>
          <button className={tab === "structure" ? "on" : ""} onClick={() => setTab("structure")}>
            Structure
          </button>
        </div>
        {tab === "data" && columns.length > 0 && (
          <button
            className="ghost"
            onClick={() => setDraft([...draft, { column: columns[0].name, op: "contains", value: "" }])}
          >
            + Filter
          </button>
        )}
        <span className="grow" />
        {tab === "data" && (
          <ExportMenu
            disabled={!meta}
            onDone={setNotice}
            note={filters.length > 0 ? "All rows that match the filters." : "All rows, in the grid's order."}
            items={[
              { label: "CSV…", run: () => exportTable("csv") },
              { label: "JSON…", run: () => exportTable("json") },
              { label: "INSERT statements…", run: () => exportTable("insert") },
            ]}
          />
        )}
      </div>

      {tab === "data" && draft.length > 0 && (
        <div className="filters">
          {draft.map((f, i) => (
            <div className="filter" key={i}>
              <select value={f.column} onChange={(e) => updateDraft(i, { column: e.target.value }, true)}>
                {columns.map((c) => (
                  <option key={c.name}>{c.name}</option>
                ))}
              </select>
              <select value={f.op} onChange={(e) => updateDraft(i, { op: e.target.value as FilterOp }, true)}>
                {OPS.map((o) => (
                  <option key={o.op} value={o.op}>
                    {o.label}
                  </option>
                ))}
              </select>
              {needsValue(f.op) && (
                <input
                  autoFocus
                  spellCheck={false}
                  placeholder="value, ↵ to apply"
                  value={f.value ?? ""}
                  onChange={(e) => updateDraft(i, { value: e.target.value })}
                  onKeyDown={(e) => e.key === "Enter" && applyFilters()}
                />
              )}
              <button
                className="ghost"
                onClick={() => {
                  const next = draft.filter((_, j) => j !== i);
                  setDraft(next);
                  applyFilters(next);
                }}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      {error && <p className="error">{error}</p>}

      {tab === "data" ? (
        meta && (
          <Grid
            columns={meta.columns}
            rowCount={Math.min(rowCount - base, WINDOW_ROWS)}
            rowOffset={base}
            row={(i) => pages[Math.floor((base + i) / PAGE_SIZE)]?.[(base + i) % PAGE_SIZE]}
            sample={pages[0] ?? []}
            onRange={onRange}
            sort={sort}
            onSort={setSort}
            scrollTo={jump}
          />
        )
      ) : (
        info && <Structure info={info} relation={relation} onOpen={onOpen} />
      )}

      {tab === "data" && meta && (
        <footer className="status pager" title={meta.sql}>
          <button className="ghost" disabled={currentPage <= 1} onClick={() => goTo(currentPage - 1)}>
            ‹
          </button>
          <span>
            Page{" "}
            <input
              className="page-input"
              key={currentPage}
              defaultValue={currentPage}
              inputMode="numeric"
              onKeyDown={(e) => {
                if (e.key === "Enter") goTo(Number(e.currentTarget.value.replace(/\D/g, "")) || 1);
              }}
            />{" "}
            of {exactTotal === null ? "~" : ""}
            {pageCount.toLocaleString()}
          </span>
          <button className="ghost" disabled={currentPage >= pageCount} onClick={() => goTo(currentPage + 1)}>
            ›
          </button>
          <span className="sep">·</span>
          <span>{totalLabel(count, exactTotal)}</span>
          <span className="sep">·</span>
          <span>{meta.elapsed_ms.toFixed(1)} ms</span>
          {notice && (
            <>
              <span className="grow" />
              <span className="ellipsis" onClick={() => setNotice(null)}>
                {notice}
              </span>
            </>
          )}
        </footer>
      )}
    </div>
  );
}

function totalLabel(count: Count | "counting" | null, exactTotal: number | null) {
  const rows = (n: number) => `${n.toLocaleString()} ${n === 1 ? "row" : "rows"}`;
  if (exactTotal !== null) return rows(exactTotal);
  if (count === "counting") return "counting…";
  if (count?.rows != null) return `~${rows(count.rows)} (estimate)`;
  return "too many rows to count quickly";
}
