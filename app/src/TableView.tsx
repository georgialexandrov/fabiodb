import { useCallback, useEffect, useRef, useState } from "react";
import { ColumnFilter, filterText } from "./ColumnFilter";
import { chooseFile, ExportMenu } from "./ExportMenu";
import { Grid, type GridEdit } from "./Grid";
import { NewRow } from "./NewRow";
import { Structure } from "./Structure";
import {
  api,
  fileName,
  plural,
  type ColumnValue,
  type Count,
  type ExportFormat,
  type Filter,
  type Page,
  type Relation,
  type RelationRef,
  type Changes,
  type Rows,
  type Sort,
  type TableInfo,
} from "./api";

const PAGE_SIZE = 100;
// WebKit can't lay out elements much taller than ~33M px (1.3M rows at 26 px).
// Bigger results scroll within a window of this many rows that moves on jumps.
const WINDOW_ROWS = 500_000;

type Props = { connectionId: string; relation: Relation; onOpen: (r: RelationRef) => void };

/** Unsaved edits of one row, keyed by column name. */
type PendingRow = { key: ColumnValue[]; changes: Record<string, { old: string | null; new: string | null }> };

export function TableView({ connectionId, relation, onOpen }: Props) {
  const [tab, setTab] = useState<"data" | "structure">("data");
  const [info, setInfo] = useState<TableInfo | null>(null);
  const [sort, setSort] = useState<Sort | null>(null);
  // The column whose filter popover is open, and where its header mark is.
  const [filtering, setFiltering] = useState<{ column: string; anchor: DOMRect } | null>(null);
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
  // Keyed by the row's primary-key values, so edits survive sorting and paging.
  const [edits, setEdits] = useState<Record<string, PendingRow>>({});
  const [inserts, setInserts] = useState<ColumnValue[][]>([]);
  // Rows marked for deletion, by the same key as edits.
  const [deletes, setDeletes] = useState<Record<string, ColumnValue[]>>({});
  const [adding, setAdding] = useState(false);
  const [review, setReview] = useState<{ sql: string[]; error: string | null; saving: boolean } | null>(null);
  const [reloads, setReloads] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const generation = useRef(0);
  const requested = useRef(new Set<number>());

  useEffect(() => {
    setSort(null);
    setFiltering(null);
    setFilters([]);
    setInfo(null);
    setEdits({});
    setInserts([]);
    setDeletes({});
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
    [connectionId, relation.schema, relation.name, sort, filters, reloads],
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

  const columns = info?.columns ?? [];
  const pkIndexes = columns.flatMap((c, i) => (c.primary_key ? [i] : []));
  const rowAt = (i: number) => pages[Math.floor((base + i) / PAGE_SIZE)]?.[(base + i) % PAGE_SIZE];
  const keyOf = (r: (string | null)[]) => JSON.stringify(pkIndexes.map((i) => r[i]));
  const editCount = Object.values(edits).reduce((n, e) => n + Object.keys(e.changes).length, 0);
  const deleteCount = Object.keys(deletes).length;
  const changeCount = editCount + inserts.length + deleteCount;
  const keyValues = (r: (string | null)[]) => pkIndexes.map((p) => ({ column: columns[p].name, value: r[p] }));
  const discardAll = () => {
    setEdits({});
    setInserts([]);
    setDeletes({});
  };

  const edit: GridEdit = {
    blocked:
      relation.kind !== "table"
        ? `${relation.name} is a view. Edit the table it reads from.`
        : info && pkIndexes.length === 0
          ? `${relation.schema}.${relation.name} has no primary key, so a row can't be picked out safely. Use an UPDATE in a query tab.`
          : null,
    onBlocked: setNotice,
    nullable: columns.map((c) => c.nullable),
    deleted(i) {
      const r = rowAt(i);
      return !!r && keyOf(r) in deletes;
    },
    onDeleteRows(rows) {
      const loaded = rows.map(rowAt).filter((r): r is (string | null)[] => !!r);
      setDeletes((all) => {
        // Marking rows that are all marked already unmarks them.
        const allMarked = loaded.every((r) => keyOf(r) in all);
        const next = { ...all };
        for (const r of loaded) {
          if (allMarked) delete next[keyOf(r)];
          else next[keyOf(r)] = keyValues(r);
        }
        return next;
      });
    },
    pending(i, c) {
      const r = rowAt(i);
      const change = r && edits[keyOf(r)]?.changes[columns[c].name];
      return change ? change.new : undefined;
    },
    onEdit(i, c, value) {
      const r = rowAt(i);
      if (!r) return;
      const key = keyOf(r);
      const name = columns[c].name;
      setEdits((all) => {
        const row: PendingRow = all[key] ?? { key: keyValues(r), changes: {} };
        const changes = { ...row.changes };
        // Back to what was loaded = no change.
        if (value === r[c]) delete changes[name];
        else changes[name] = { old: r[c], new: value };
        const { [key]: _, ...rest } = all;
        return Object.keys(changes).length ? { ...rest, [key]: { ...row, changes } } : rest;
      });
    },
  };

  const changes = (): Changes => ({
    // A row being deleted doesn't need its edits.
    updates: Object.entries(edits)
      .filter(([key]) => !(key in deletes))
      .map(([, e]) => ({
        key: e.key,
        changes: Object.entries(e.changes).map(([column, c]) => ({ column, old: c.old, new: c.new })),
      })),
    inserts,
    deletes: Object.values(deletes),
  });

  async function startReview() {
    if (changeCount === 0) return;
    try {
      setReview({ sql: await api.previewChanges(connectionId, relation, changes()), error: null, saving: false });
    } catch (e) {
      setNotice(String(e));
    }
  }

  async function save() {
    if (!review) return;
    setReview({ ...review, saving: true, error: null });
    const started = performance.now();
    try {
      const rows = await api.applyChanges(connectionId, relation, changes());
      discardAll();
      setReview(null);
      setReloads((n) => n + 1);
      setNotice(`Saved ${plural(rows, "statement")} · ${Math.round(performance.now() - started)} ms`);
    } catch (e) {
      setReview({ ...review, saving: false, error: String(e) });
    }
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!root.current?.offsetParent) return;
      if (review && !review.saving && (e.key === "Enter" || e.key === "Escape")) {
        // Buttons don't reliably take focus in WebKit, so the dialog listens itself.
        e.preventDefault();
        if (e.key === "Enter") save();
        else setReview(null);
      } else if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        startReview();
      } else if ((e.metaKey || e.ctrlKey) && e.key === "r") {
        // Reload rows and the count. Unsaved edits stay, still checked against what was loaded.
        e.preventDefault();
        setReloads((n) => n + 1);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /** Every row under the current sort and filters, streamed to a file by the core. */
  async function exportTable(format: ExportFormat) {
    const path = await chooseFile(relation.name, format);
    if (!path) return null;
    const started = performance.now();
    const rows = await api.exportTable(connectionId, relation, sort, filters, format, path);
    return `Saved ${plural(rows, "row")} to ${fileName(path)} · ${((performance.now() - started) / 1000).toFixed(1)} s`;
  }

  return (
    <div className="table-view" ref={root}>
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
        <span className="grow" />
        {tab === "data" && !edit.blocked && info && (
          <button className="ghost" onClick={() => setAdding(true)} title="Add a row; saved with ⌘S">
            + Row
          </button>
        )}
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

      {tab === "data" && filters.length > 0 && (
        <div className="filter-chips">
          {filters.map((f, i) => (
            <span key={i} className="chip">
              <button
                className="chip-text"
                onClick={(e) => setFiltering({ column: f.column, anchor: e.currentTarget.getBoundingClientRect() })}
              >
                {filterText(f)}
              </button>
              <button className="chip-remove" onClick={() => setFilters(filters.filter((_, j) => j !== i))} title="Remove">
                ✕
              </button>
            </span>
          ))}
          {filters.length > 1 && (
            <button className="ghost" onClick={() => setFilters([])}>
              Clear all
            </button>
          )}
        </div>
      )}

      {filtering && columns.some((c) => c.name === filtering.column) && (
        <ColumnFilter
          column={columns.find((c) => c.name === filtering.column)!}
          anchor={filtering.anchor}
          applied={filters.filter((f) => f.column === filtering.column)}
          onAdd={(f) => setFilters([...filters, f])}
          onRemove={(f) => setFilters(filters.filter((x) => x !== f))}
          onClose={() => setFiltering(null)}
        />
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
            edit={edit}
            table={relation}
            onNotice={setNotice}
            filtered={(c) => filters.some((f) => f.column === meta.columns[c].name)}
            onFilter={(c, anchor) => setFiltering({ column: meta.columns[c].name, anchor })}
          />
        )
      ) : (
        info && <Structure info={info} relation={relation} onOpen={onOpen} />
      )}

      {tab === "data" && changeCount > 0 && (
        <div className="edit-bar">
          <span>
            {[
              editCount && plural(editCount, "changed value"),
              inserts.length && plural(inserts.length, "new row"),
              deleteCount && `${plural(deleteCount, "row")} to delete`,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
          <span className="grow" />
          <button className="ghost" onClick={discardAll}>
            Discard
          </button>
          <button className="primary" onClick={startReview} title="Review the statements (⌘S)">
            Save…
          </button>
        </div>
      )}

      {adding && (
        <NewRow
          table={`${relation.schema}.${relation.name}`}
          columns={columns}
          onClose={() => setAdding(false)}
          onAdd={(row) => {
            setInserts((all) => [...all, row]);
            setAdding(false);
          }}
        />
      )}

      {review && (
        <div className="modal-backdrop" onMouseDown={() => !review.saving && setReview(null)}>
          <div
            className="modal wide"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <h2>
              Save to {relation.schema}.{relation.name}?
            </h2>
            <p className="muted">
              Runs in one transaction. If any row changed or went away since it was loaded, nothing is saved.
            </p>
            <pre className="sql-preview">{review.sql.join("\n")}</pre>
            {review.error && <p className="error">{review.error}</p>}
            <div className="actions">
              <span className="grow" />
              <button onClick={() => setReview(null)} disabled={review.saving}>
                Cancel
              </button>
              <button className="primary" onClick={save} disabled={review.saving} title="↵">
                {review.saving ? "Saving…" : "Save"}
              </button>
            </div>
          </div>
        </div>
      )}

      {tab === "data" && meta && (
        <footer className="status pager" title={`${meta.sql}\n⌘R reloads`}>
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
