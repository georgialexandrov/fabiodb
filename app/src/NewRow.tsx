import { useState } from "react";
import type { Column, ColumnValue } from "./api";

type Props = {
  table: string;
  columns: Column[];
  onAdd: (values: ColumnValue[]) => void;
  onClose: () => void;
};

/** A new row: empty fields take the column's default, NULL sets NULL. It's added to the pending save. */
export function NewRow({ table, columns, onAdd, onClose }: Props) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [nulls, setNulls] = useState<Set<string>>(new Set());

  function add() {
    const row: ColumnValue[] = [];
    for (const c of columns) {
      if (nulls.has(c.name)) row.push({ column: c.name, value: null });
      else if ((values[c.name] ?? "") !== "") row.push({ column: c.name, value: values[c.name] });
    }
    onAdd(row);
  }

  const toggleNull = (name: string) =>
    setNulls((n) => {
      const next = new Set(n);
      if (!next.delete(name)) next.add(name);
      return next;
    });

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <form
        className="modal wide"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <h2>New row in {table}</h2>
        <p className="muted">Empty fields get the column’s default. Nothing is saved until you review it with ⌘S.</p>
        <div className="new-row-fields">
          {columns.map((c, i) => (
            <label key={c.name} className="row-field">
              <span className="row-field-name">
                {c.name} <span className="muted">{c.data_type}</span>
                {c.primary_key && <span className="muted"> · key</span>}
              </span>
              <div className="row-field-edit">
                <textarea
                  autoFocus={i === 0}
                  spellCheck={false}
                  rows={1}
                  disabled={nulls.has(c.name)}
                  placeholder={nulls.has(c.name) ? "NULL" : (c.default ?? (c.nullable ? "NULL" : "DEFAULT"))}
                  value={values[c.name] ?? ""}
                  onChange={(e) => setValues({ ...values, [c.name]: e.target.value })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      e.preventDefault();
                      add();
                    }
                  }}
                />
                {c.nullable && (
                  <button
                    type="button"
                    className={`ghost set-null-field ${nulls.has(c.name) ? "on" : ""}`}
                    onClick={() => toggleNull(c.name)}
                    title="Set to NULL"
                  >
                    NULL
                  </button>
                )}
              </div>
            </label>
          ))}
        </div>
        <div className="actions">
          <span className="grow" />
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="primary" title="⌘↵">
            Add row
          </button>
        </div>
      </form>
    </div>
  );
}
