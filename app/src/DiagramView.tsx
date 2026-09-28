import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api, type Diagram, type RelationRef, type SchemaTable } from "./api";
import { HEADER, ROW, WIDTH, edgePath, layOut, schemaFrames, tableHeight, tableKey, type Point } from "./diagramLayout";

type Props = {
  connectionId: string;
  /** The schema picked in the sidebar: only its tables are shown. */
  only: string | null;
  visible: boolean;
  onOpen: (r: RelationRef) => void;
};

const MARGIN = 40;
const clampZoom = (z: number) => Math.min(2, Math.max(0.2, z));
const fileName = (path: string) => path.split("/").pop() ?? path;

/** The tables and their references, from the live schema. Drag a table to move it; double-click opens it. */
export function DiagramView({ connectionId, only, visible, onOpen }: Props) {
  const [diagram, setDiagram] = useState<Diagram | null>(null);
  const [positions, setPositions] = useState<Record<string, Point>>({});
  const [zoom, setZoom] = useState(1);
  const [hover, setHover] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  // Read by drag, which memoized cards keep from their last render.
  const live = useRef({ positions, zoom });
  live.current = { positions, zoom };
  // The point that stays under the pointer while zooming: canvas and screen coordinates.
  const anchor = useRef<{ x: number; y: number; cx: number; cy: number } | null>(null);

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

  /** Zooms keeping the point under the pointer (or the middle of the view) where it is. */
  function zoomTo(next: number, clientX?: number, clientY?: number) {
    const z = live.current.zoom;
    const target = clampZoom(next);
    if (!scroller || target === z) return;
    const r = scroller.getBoundingClientRect();
    const cx = (clientX ?? r.left + r.width / 2) - r.left;
    const cy = (clientY ?? r.top + r.height / 2) - r.top;
    anchor.current = { x: (scroller.scrollLeft + cx) / z, y: (scroller.scrollTop + cy) / z, cx, cy };
    live.current.zoom = target;
    setZoom(target);
  }
  const zoomAt = useRef(zoomTo);
  zoomAt.current = zoomTo;

  // Scroll so the anchor stays put, before the frame is painted.
  useLayoutEffect(() => {
    const a = anchor.current;
    if (!a || !scroller) return;
    anchor.current = null;
    scroller.scrollLeft = a.x * zoom - a.cx;
    scroller.scrollTop = a.y * zoom - a.cy;
  }, [zoom, scroller]);

  // Pinch (WebKit gesture events; ctrl + wheel elsewhere), and ⌘= ⌘- ⌘0.
  useEffect(() => {
    if (!visible || !scroller) return;
    type Gesture = UIEvent & { scale: number; clientX: number; clientY: number };
    let startZoom = 1;
    // WebKit may also send ctrl + wheel for the same pinch; count it once.
    let pinching = false;
    const gestureStart = (e: Event) => {
      e.preventDefault();
      pinching = true;
      startZoom = live.current.zoom;
    };
    const gestureChange = (e: Event) => {
      e.preventDefault();
      const g = e as Gesture;
      zoomAt.current(startZoom * g.scale, g.clientX, g.clientY);
    };
    const gestureEnd = (e: Event) => {
      e.preventDefault();
      pinching = false;
    };
    const wheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      if (pinching) return;
      zoomAt.current(live.current.zoom * Math.exp(-e.deltaY / 100), e.clientX, e.clientY);
    };
    const key = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const z = live.current.zoom;
      if (e.key === "=" || e.key === "+") zoomAt.current(z * 1.25);
      else if (e.key === "-") zoomAt.current(z / 1.25);
      else if (e.key === "0") zoomAt.current(1);
      else return;
      e.preventDefault();
    };
    scroller.addEventListener("gesturestart", gestureStart);
    scroller.addEventListener("gesturechange", gestureChange);
    scroller.addEventListener("gestureend", gestureEnd);
    scroller.addEventListener("wheel", wheel, { passive: false });
    window.addEventListener("keydown", key);
    return () => {
      scroller.removeEventListener("gesturestart", gestureStart);
      scroller.removeEventListener("gesturechange", gestureChange);
      scroller.removeEventListener("gestureend", gestureEnd);
      scroller.removeEventListener("wheel", wheel);
      window.removeEventListener("keydown", key);
    };
  }, [visible, scroller]);

  /** Dragging the empty canvas moves the view, like a map. */
  function pan(e: React.PointerEvent) {
    if (e.button !== 0 || !scroller || (e.target as Element).closest(".diagram-table")) return;
    e.preventDefault();
    const [x0, y0, left, top] = [e.clientX, e.clientY, scroller.scrollLeft, scroller.scrollTop];
    scroller.classList.add("panning");
    const move = (m: PointerEvent) => {
      scroller.scrollLeft = left - (m.clientX - x0);
      scroller.scrollTop = top - (m.clientY - y0);
    };
    const up = () => {
      scroller.classList.remove("panning");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  const schema = diagram?.schema;
  // Every table is laid out; a picked schema only narrows what's drawn.
  const tables = useMemo(
    () =>
      new Map(
        (schema?.tables ?? []).filter((t) => !only || t.schema === only).map((t) => [tableKey(schema!.engine, t), t]),
      ),
    [schema, only],
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
    e.preventDefault();
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
      // Only this table moves while dragging, so the next layout is known here, not in a state updater.
      latest = { ...latest, [key]: [Math.max(0, Math.round(start[0] + dx)), Math.max(0, Math.round(start[1] + dy))] };
      setPositions(latest);
    };
    document.body.classList.add("dragging");
    const up = () => {
      document.body.classList.remove("dragging");
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

  const frames = only ? [] : schemaFrames(schema, positions);
  // What's drawn starts at the margin, however far its tables sit in the full layout.
  const boxes = [
    ...[...tables].flatMap(([k, t]) => (positions[k] ? [[...positions[k], WIDTH, tableHeight(t)]] : [])),
    ...frames.map((f) => [f.x, f.y, f.width, f.height]),
  ];
  const origin: Point = boxes.length ? [Math.min(...boxes.map((b) => b[0])), Math.min(...boxes.map((b) => b[1]))] : [0, 0];
  const shift = ([x, y]: Point): Point => [x - origin[0] + MARGIN, y - origin[1] + MARGIN];
  let width = 0;
  let height = 0;
  for (const [x, y, w, h] of boxes) {
    width = Math.max(width, x - origin[0] + w);
    height = Math.max(height, y - origin[1] + h);
  }
  width += MARGIN * 2 + 64;
  height += MARGIN * 2;
  const related = hover ? new Set(edges.flatMap((e) => (e.from === hover || e.to === hover ? [e.from, e.to] : []))) : null;
  const linked = diagram.dbml ? fileName(diagram.dbml) : null;

  return (
    <div className="diagram-view">
      <div className="toolbar" data-tauri-drag-region>
        <span className="muted">
          {only && `${only} · `}
          {tables.size.toLocaleString()} {tables.size === 1 ? "table" : "tables"} · {edges.length.toLocaleString()}{" "}
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
        <button className="ghost zoom" title="Pinch or ⌘= ⌘- to zoom; click for 100% (⌘0)" onClick={() => zoomTo(1)}>
          {Math.round(zoom * 100)}%
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {tables.size === 0 ? (
        <p className="hint">No tables here.</p>
      ) : (
        <div className="diagram-scroll" ref={setScroller} onPointerDown={pan}>
          <div style={{ width: width * zoom, height: height * zoom }}>
            <div className="diagram-canvas" style={{ width, height, transform: `scale(${zoom})` }}>
              {frames.map((f) => {
                const [x, y] = shift([f.x, f.y]);
                return (
                  <div key={f.schema} className="diagram-frame" style={{ left: x, top: y, width: f.width, height: f.height }}>
                    <span>{f.schema}</span>
                  </div>
                );
              })}
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
