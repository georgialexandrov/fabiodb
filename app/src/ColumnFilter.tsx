import { useEffect, useRef, useState } from "react";
import { isNumeric, type Column, type Filter, type FilterOp } from "./api";

export const OPS: { op: FilterOp; label: string }[] = [
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

export const needsValue = (op: FilterOp) => op !== "is_null" && op !== "is_not_null";

export const opLabel = (op: FilterOp) => OPS.find((o) => o.op === op)?.label ?? op;

/** `name contains "rock"`, as a chip shows it. */
export function filterText(f: Filter) {
  return needsValue(f.op) ? `${f.column} ${opLabel(f.op)} ${JSON.stringify(f.value ?? "")}` : `${f.column} ${opLabel(f.op)}`;
}

type Props = {
  column: Column;
  /** Where the header's filter mark is, to sit under it. */
  anchor: DOMRect;
  /** Conditions already applied to this column. */
  applied: Filter[];
  onAdd: (filter: Filter) => void;
  onRemove: (filter: Filter) => void;
  onClose: () => void;
};

/** One column's conditions: the applied ones, and a row to add another. ↵ applies. */
export function ColumnFilter({ column, anchor, applied, onAdd, onRemove, onClose }: Props) {
  const [op, setOp] = useState<FilterOp>(isNumeric(column.data_type) ? "eq" : "contains");
  const [value, setValue] = useState("");
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !root.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [onClose]);

  function add() {
    if (needsValue(op) && value === "") return;
    onAdd({ column: column.name, op, value: needsValue(op) ? value : null });
    setValue("");
  }

  // Under the mark, kept inside the window.
  const left = Math.max(8, Math.min(anchor.left - 12, window.innerWidth - 340));
  return (
    <div className="column-filter" ref={root} style={{ left, top: anchor.bottom + 6 }}>
      <div className="column-filter-title">
        {column.name} <span className="muted">{column.data_type}</span>
      </div>
      {applied.map((f, i) => (
        <div key={i} className="column-filter-applied">
          <span className="ellipsis">
            {opLabel(f.op)} {needsValue(f.op) && <code>{f.value}</code>}
          </span>
          <button className="ghost" onClick={() => onRemove(f)} title="Remove this condition">
            ✕
          </button>
        </div>
      ))}
      <div className="column-filter-add">
        <select value={op} onChange={(e) => setOp(e.target.value as FilterOp)}>
          {OPS.filter((o) => column.nullable || (o.op !== "is_null" && o.op !== "is_not_null")).map((o) => (
            <option key={o.op} value={o.op}>
              {o.label}
            </option>
          ))}
        </select>
        {needsValue(op) && (
          <input
            autoFocus
            spellCheck={false}
            placeholder="value"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add()}
          />
        )}
        <button className="primary" onClick={add} disabled={needsValue(op) && value === ""}>
          Add
        </button>
      </div>
    </div>
  );
}
