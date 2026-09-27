import { memo, useEffect, useMemo, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api, type Diagram, type RelationRef, type SchemaTable } from "./api";
import { HEADER, ROW, WIDTH, edgePath, layOut, tableHeight, tableKey, type Point } from "./diagramLayout";

type Props = { connectionId: string; visible: boolean; onOpen: (r: RelationRef) => void };

const MARGIN = 40;
const clampZoom = (z: number) => Math.min(2, Math.max(0.25, Math.round(z * 100) / 100));
const fileName = (path: string) => path.split("/").pop() ?? path;

/** The tables and their references, from the live schema. Drag a table to move it; double-click opens it. */
export function DiagramView({ connectionId, visible, onOpen }: Props) {
  const [diagram, setDiagram] = useState<Diagram | null>(null);
  const [positions, setPositions] = useState<Record<string, Point>>({});
  const [zoom, setZoom] = useState(1);
  const [hover, setHover] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  // Read by drag, which memoized cards keep from their last render.
  const live = useRef({ positions, zoom });
  live.current = { positions, zoom };

  async function load() {
    try {
      const d = await api.diagram(connectionId);
      setDiagram(d);
      setPositions(layOut(d.schema, d.layout.tables));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  // Reread when the tab comes back: the schema may have changed meanwhile.
  useEffect(() => {
    if (visible) load();
  }, [connectionId, visible]);

  // ⌘= ⌘- ⌘0, and pinch (ctrl + wheel), while on screen.
  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "=" || e.key === "+") setZoom((z) => clampZoom(z * 1.2));
      else if (e.key === "-") setZoom((z) => clampZoom(z / 1.2));
      else if (e.key === "0") setZoom(1);
      else return;
      e.preventDefault();
    };
    const el = scroller.current;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      setZoom((z) => clampZoom(z * Math.exp(-e.deltaY / 200)));
    };
    window.addEventListener("keydown", onKey);
    el?.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      window.removeEventListener("keydown", onKey);
      el?.removeEventListener("wheel", onWheel);
    };
  }, [visible]);

  const schema = diagram?.schema;
  const tables = useMemo(
    () => new Map((schema?.tables ?? []).map((t) => [tableKey(schema!.engine, t), t])),
    [schema],
  );
  const edges = useMemo(() => {
    if (!schema) return [];
    return schema.tables.flatMap((t) =>
      t.foreign_keys.map((fk) => {
        const to = tableKey(schema.engine, { schema: fk.ref_schema, name: fk.ref_table });
        const target = tables.get(to);
        return {
          from: tableKey(schema.engine, t),
          fromRow: Math.max(0, t.columns.findIndex((c) => c.name === fk.columns[0])),
          to,
          toRow: Math.max(0, target?.columns.findIndex((c) => c.name === fk.ref_columns[0]) ?? 0),
        };
      }),
    ).filter((e) => tables.has(e.to));
  }, [schema, tables]);

  function persist(next: Record<string, Point>) {
    api.saveLayout(connectionId, { tables: next }).catch((e) => setError(String(e)));
  }

  function drag(key: string, e: React.PointerEvent) {
    if (e.button !== 0) return;
    const { positions: before, zoom: z } = live.current;
    const start = before[key];
    const [x0, y0] = [e.clientX, e.clientY];
    let moved = false;
    let latest = before;
    const move = (m: PointerEvent) => {
      const dx = (m.clientX - x0) / z;
      const dy = (m.clientY - y0) / z;
      if (!moved && Math.abs(dx) + Math.abs(dy) < 3) return;
      moved = true;
      setPositions((p) => (latest = { ...p, [key]: [Math.max(0, Math.round(start[0] + dx)), Math.max(0, Math.round(start[1] + dy))] }));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      // Written once, when the table lands: one line changes in the file.
      if (moved) persist(latest);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function arrange() {
    if (!schema) return;
    const next = layOut(schema, {});
    setPositions(next);
    persist(next);
  }

  async function exportDbml() {
    const path = await save({
      defaultPath: diagram?.dbml ?? "schema.dbml",
      filters: [{ name: "DBML", extensions: ["dbml"] }],
    });
    if (!path) return;
    try {
      await api.saveLayout(connectionId, { tables: positions });
      await api.exportDbml(connectionId, path);
      setStatus(`Saved ${fileName(path)} and ${fileName(path).replace(/\.dbml$/, "")}.layout.json`);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function update() {
    if (!diagram?.dbml) return;
    try {
      await api.exportDbml(connectionId, diagram.dbml);
      setStatus(`Updated ${fileName(diagram.dbml)}`);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function forget() {
    await api.forgetDbml(connectionId).catch((e) => setError(String(e)));
    await load();
  }

  if (error && !diagram) return <p className="error">{error}</p>;
  if (!diagram || !schema) return <p className="hint">Reading the schema…</p>;

  let width = 0;
  let height = 0;
  for (const [k, [x, y]] of Object.entries(positions)) {
    const t = tables.get(k);
    if (!t) continue;
    width = Math.max(width, x + WIDTH);
    height = Math.max(height, y + tableHeight(t));
  }
  width += MARGIN * 2 + 64;
  height += MARGIN * 2;
  const related = hover ? new Set(edges.flatMap((e) => (e.from === hover || e.to === hover ? [e.from, e.to] : []))) : null;
  const linked = diagram.dbml ? fileName(diagram.dbml) : null;

  return (
    <div className="diagram-view">
      <div className="toolbar" data-tauri-drag-region>
        <span className="muted">
          {schema.tables.length.toLocaleString()} {schema.tables.length === 1 ? "table" : "tables"} · {edges.length.toLocaleString()}{" "}
          {edges.length === 1 ? "reference" : "references"}
        </span>
        <span className="grow" />
        {diagram.missing ? (
          <>
            <span className="accent">{linked} was moved or deleted.</span>
            <button className="ghost" onClick={forget}>
              Forget
            </button>
          </>
        ) : diagram.stale ? (
          <>
            <span className="accent" title={diagram.dbml!}>
              {linked} no longer matches the database.
            </span>
            <button onClick={update}>Update</button>
          </>
        ) : (
          linked && (
            <span className="muted" title={diagram.dbml!}>
              {linked}
            </span>
          )
        )}
        <button className="ghost" onClick={arrange} title="Lay out every table again">
          Arrange
        </button>
        <button className="ghost" onClick={exportDbml} title="Save the schema as DBML, with the layout next to it">
          Export DBML…
        </button>
        <span className="muted zoom" title="⌘= ⌘- ⌘0, or pinch">
          {Math.round(zoom * 100)}%
        </span>
      </div>
      {error && <p className="error">{error}</p>}
      {schema.tables.length === 0 ? (
        <p className="hint">No tables here.</p>
      ) : (
        <div className="diagram-scroll" ref={scroller}>
          <div style={{ width: width * zoom, height: height * zoom }}>
            <div className="diagram-canvas" style={{ width, height, transform: `scale(${zoom})` }}>
              <svg className="diagram-edges" width={width} height={height}>
                {edges.map((e, i) => {
                  const from = positions[e.from];
                  const to = positions[e.to];
                  if (!from || !to) return null;
                  const on = hover !== null && (e.from === hover || e.to === hover);
                  return (
                    <path
                      key={i}
                      className={on ? "on" : ""}
                      d={edgePath(shift(from), e.fromRow, shift(to), e.toRow)}
                    />
                  );
                })}
              </svg>
              {[...tables].map(([k, t]) =>
                positions[k] ? (
                  <TableCard
                    key={k}
                    name={k}
                    table={t}
                    at={shift(positions[k])}
                    dim={related !== null && !related.has(k)}
                    onHover={setHover}
                    onDrag={drag}
                    onOpen={onOpen}
                  />
                ) : null,
              )}
            </div>
          </div>
        </div>
      )}
      {status && <div className="status">{status}</div>}
    </div>
  );
}

const shift = ([x, y]: Point): Point => [x + MARGIN, y + MARGIN];

type CardProps = {
  name: string;
  table: SchemaTable;
  at: Point;
  dim: boolean;
  onHover: (key: string | null) => void;
  onDrag: (key: string, e: React.PointerEvent) => void;
  onOpen: (r: RelationRef) => void;
};

/** Memoized: dragging one table re-renders that one only. */
const TableCard = memo(function TableCard({ name, table, at, dim, onHover, onDrag, onOpen }: CardProps) {
  const fks = new Set(table.foreign_keys.flatMap((fk) => fk.columns));
  return (
    <div
      className={`diagram-table ${dim ? "dim" : ""}`}
      style={{ left: at[0], top: at[1], width: WIDTH }}
      onPointerEnter={() => onHover(name)}
      onPointerLeave={() => onHover(null)}
    >
      <div
        className="diagram-table-name ellipsis"
        style={{ height: HEADER }}
        title={table.comment ?? "Double-click to open"}
        onPointerDown={(e) => onDrag(name, e)}
        onDoubleClick={() => onOpen({ schema: table.schema, name: table.name })}
      >
        {name}
      </div>
      {table.columns.map((c) => (
        <div key={c.name} className="diagram-column" style={{ height: ROW }} title={c.comment ?? undefined}>
          <span className={`ellipsis ${c.primary_key ? "pk" : ""}`}>
            {c.name}
            {fks.has(c.name) && <span className="muted"> →</span>}
          </span>
          <span className="muted ellipsis">{c.data_type}</span>
        </div>
      ))}
    </div>
  );
}, (a, b) => a.at[0] === b.at[0] && a.at[1] === b.at[1] && a.dim === b.dim && a.table === b.table);
