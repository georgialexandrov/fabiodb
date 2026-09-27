import type { Schema, SchemaTable } from "./api";

// Fixed sizes: no text measuring, so layout is arithmetic.
export const WIDTH = 240;
export const HEADER = 30;
export const ROW = 20;
const PAD = 6;
const GAP_X = 96;
const GAP_Y = 28;
/** A column of tables taller than this wraps into the next one. */
const MAX_COLUMN = 1400;

export type Point = [number, number];

/** As DBML names it: the default schema is left out. */
export function tableKey(engine: Schema["engine"], t: { schema: string; name: string }): string {
  return t.schema === (engine === "postgres" ? "public" : "main") ? t.name : `${t.schema}.${t.name}`;
}

export function tableHeight(t: SchemaTable): number {
  return HEADER + t.columns.length * ROW + PAD;
}

/**
 * Positions for every table. Saved ones stay where they are; the rest are laid
 * out left to right by reference depth (a table sits right of what it
 * references), then placed below anything saved. Linear in tables + keys.
 */
export function layOut(schema: Schema, saved: Record<string, Point>): Record<string, Point> {
  const key = (t: { schema: string; name: string }) => tableKey(schema.engine, t);
  const byKey = new Map(schema.tables.map((t) => [key(t), t]));
  const out: Record<string, Point> = {};
  for (const [k, p] of Object.entries(saved)) if (byKey.has(k)) out[k] = p;
  const fresh = schema.tables.filter((t) => !(key(t) in out));
  if (fresh.length === 0) return out;

  const parents = new Map<string, string[]>();
  const linked = new Set<string>();
  for (const t of schema.tables) {
    const refs = t.foreign_keys
      .map((fk) => key({ schema: fk.ref_schema, name: fk.ref_table }))
      .filter((r) => r !== key(t) && byKey.has(r));
    parents.set(key(t), refs);
    if (refs.length) linked.add(key(t));
    refs.forEach((r) => linked.add(r));
  }

  // Depth = longest chain of references below a table. A cycle counts once.
  const depth = new Map<string, number>();
  const visiting = new Set<string>();
  const depthOf = (k: string): number => {
    const known = depth.get(k);
    if (known !== undefined) return known;
    if (visiting.has(k)) return 0;
    visiting.add(k);
    const d = Math.max(-1, ...(parents.get(k) ?? []).map(depthOf)) + 1;
    visiting.delete(k);
    depth.set(k, d);
    return d;
  };

  const ranks: string[][] = [];
  const loners: string[] = [];
  for (const t of fresh) {
    const k = key(t);
    if (!linked.has(k)) loners.push(k);
    else (ranks[depthOf(k)] ??= []).push(k);
  }
  if (loners.length) ranks.push(loners);

  // One sweep: order each rank by where its parents ended up, so lines cross less.
  const order = new Map<string, number>();
  const columns: string[][] = [];
  for (const rank of ranks.filter(Boolean)) {
    const weight = (k: string) => {
      const placed = (parents.get(k) ?? []).map((p) => order.get(p)).filter((o): o is number => o !== undefined);
      return placed.length ? placed.reduce((a, b) => a + b, 0) / placed.length : Infinity;
    };
    const sorted = [...rank].sort((a, b) => weight(a) - weight(b) || a.localeCompare(b));
    let column: string[] = [];
    let height = 0;
    for (const k of sorted) {
      const h = tableHeight(byKey.get(k)!) + GAP_Y;
      if (column.length && height + h > MAX_COLUMN) {
        columns.push(column);
        column = [];
        height = 0;
      }
      column.push(k);
      height += h;
    }
    columns.push(column);
    sorted.forEach((k, i) => order.set(k, i));
  }

  const top = Object.keys(out).length
    ? Math.max(...Object.entries(out).map(([k, [, y]]) => y + tableHeight(byKey.get(k)!))) + GAP_Y * 2
    : 0;
  columns.forEach((column, c) => {
    let y = top;
    for (const k of column) {
      out[k] = [c * (WIDTH + GAP_X), y];
      y += tableHeight(byKey.get(k)!) + GAP_Y;
    }
  });
  return out;
}

/** A reference from one column to another, as a curve between the table sides. */
export function edgePath(from: Point, fromRow: number, to: Point, toRow: number): string {
  const y1 = from[1] + HEADER + fromRow * ROW + ROW / 2;
  const y2 = to[1] + HEADER + toRow * ROW + ROW / 2;
  let x1: number, x2: number, bend: number;
  if (to[0] + WIDTH < from[0]) {
    [x1, x2, bend] = [from[0], to[0] + WIDTH, -1];
  } else if (from[0] + WIDTH < to[0]) {
    [x1, x2, bend] = [from[0] + WIDTH, to[0], 1];
  } else {
    // Overlapping columns: leave and enter on the right, looping out.
    const x = Math.max(from[0], to[0]) + WIDTH;
    return `M${from[0] + WIDTH},${y1} C${x + 48},${y1} ${x + 48},${y2} ${to[0] + WIDTH},${y2}`;
  }
  const dx = Math.max(32, Math.abs(x2 - x1) / 2) * bend;
  return `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}`;
}
