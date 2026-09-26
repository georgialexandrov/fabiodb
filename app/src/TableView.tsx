import { useCallback, useEffect, useRef, useState } from "react";
import { Grid } from "./Grid";
import { Structure } from "./Structure";
import {
  api,
  compactCount,
  type Filter,
  type FilterOp,
  type Page,
  type Relation,
  type RelationRef,
  type Rows,
  type Sort,
  type TableInfo,
} from "./api";

const PAGE_SIZE = 200;

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
  const [page, setPage] = useState<Omit<Page, "rows"> | null>(null);
  const [rows, setRows] = useState<Rows>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);

  useEffect(() => {
    setSort(null);
    setDraft([]);
    setFilters([]);
    setInfo(null);
    api.describe(connectionId, relation).then(setInfo, (e) => setError(String(e)));
  }, [connectionId, relation.schema, relation.name]);

  const load = useCallback(
    async (offset: number) => {
      const gen = offset === 0 ? ++generation.current : generation.current;
      setLoading(true);
      if (offset === 0) setError(null);
      try {
        const p = await api.page(connectionId, { relation, sort, filters, offset, limit: PAGE_SIZE });
        if (gen !== generation.current) return; // a newer request replaced this one
        setPage(p);
        setRows((prev) => (offset === 0 ? p.rows : [...prev, ...p.rows]));
      } catch (e) {
        if (gen !== generation.current) return;
        setError(String(e));
        if (offset === 0) setRows([]);
      } finally {
        if (gen === generation.current) setLoading(false);
      }
    },
    [connectionId, relation.schema, relation.name, sort, filters],
  );

  useEffect(() => {
    load(0);
  }, [load]);

  function applyFilters(next = draft) {
    setFilters(next.filter((f) => !needsValue(f.op) || (f.value ?? "") !== ""));
  }

  function updateDraft(i: number, patch: Partial<Filter>, apply = false) {
    const next = draft.map((f, j) => (j === i ? { ...f, ...patch } : f));
    setDraft(next);
    if (apply) applyFilters(next);
  }

  const columns = info?.columns ?? [];

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
        page && (
          <Grid
            columns={page.columns}
            rows={rows}
            hasMore={page.has_more && !loading && !error}
            onLoadMore={() => load(rows.length)}
            sort={sort}
            onSort={setSort}
          />
        )
      ) : (
        info && <Structure info={info} relation={relation} onOpen={onOpen} />
      )}

      <footer className="status" title={page?.sql}>
        {loading
          ? "Loading…"
          : page &&
            `${rows.length.toLocaleString()}${page.has_more ? "+" : ""} rows` +
              (relation.estimated_rows != null ? ` of ~${compactCount(relation.estimated_rows)}` : "") +
              ` · ${page.elapsed_ms.toFixed(1)} ms`}
      </footer>
    </div>
  );
}
