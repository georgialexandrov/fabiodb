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

/** Space above a schema's tables for its frame and name. */
export const FRAME_PAD = 16;
export const FRAME_LABEL = 24;
const GROUP_TOP = FRAME_PAD + FRAME_LABEL + 8;
const GROUP_GAP = 160;
/** Schema blocks wrap onto a new row past this width. */
const MAX_ROW = 4000;

/**
 * Positions for every table. Saved ones stay where they are. The rest are laid
 * out per schema — each schema its own block, blocks side by side — and inside
 * a block left to right by reference depth (a table sits right of what it
 * references). New tables of a schema already placed go below it; new schemas
 * below everything. Linear in tables + keys.
 */
export function layOut(schema: Schema, saved: Record<string, Point>): Record<string, Point> {
  const key = (t: { schema: string; name: string }) => tableKey(schema.engine, t);
  const byKey = new Map(schema.tables.map((t) => [key(t), t]));
  const out: Record<string, Point> = {};
  for (const [k, p] of Object.entries(saved)) if (byKey.has(k)) out[k] = p;
  const bottom = (keys: string[]) => Math.max(...keys.map((k) => out[k][1] + tableHeight(byKey.get(k)!)));

  const groups = schemaGroups(schema);
  const grouped = groups.size > 1;
  const placed = Object.keys(out);
  let x = 0;
  let y = placed.length ? bottom(placed) + GAP_Y * 2 + (grouped ? GROUP_TOP : 0) : grouped ? GROUP_TOP : 0;
  let rowHeight = 0;
  for (const tables of groups.values()) {
    const fresh = tables.filter((t) => !(key(t) in out));
    if (fresh.length === 0) continue;
    const block = arrange(fresh, key, byKey);
    const kept = tables.map(key).filter((k) => k in out);
    if (kept.length) {
      // Below the rest of its schema.
      const left = Math.min(...kept.map((k) => out[k][0]));
      const top = bottom(kept) + GAP_Y * 2;
      for (const [k, [bx, by]] of Object.entries(block.at)) out[k] = [left + bx, top + by];
      continue;
    }
    if (x > 0 && x + block.width > MAX_ROW) {
      x = 0;
      y += rowHeight + GROUP_GAP;
      rowHeight = 0;
    }
    for (const [k, [bx, by]] of Object.entries(block.at)) out[k] = [x + bx, y + by];
    x += block.width + GROUP_GAP;
    rowHeight = Math.max(rowHeight, block.height);
  }
  return out;
}

/** Tables by schema, the default schema first, then by name. */
export function schemaGroups(schema: Schema): Map<string, SchemaTable[]> {
  const home = schema.engine === "postgres" ? "public" : "main";
  const names = [...new Set(schema.tables.map((t) => t.schema))].sort((a, b) => Number(b === home) - Number(a === home) || a.localeCompare(b));
  const groups = new Map(names.map((n) => [n, [] as SchemaTable[]]));
  for (const t of schema.tables) groups.get(t.schema)!.push(t);
  return groups;
}

/** One block from (0, 0): columns by reference depth, references outside the block ignored. */
function arrange(
  tables: SchemaTable[],
  key: (t: { schema: string; name: string }) => string,
  byKey: Map<string, SchemaTable>,
): { at: Record<string, Point>; width: number; height: number } {
  const inBlock = new Set(tables.map(key));
  const parents = new Map<string, string[]>();
  const linked = new Set<string>();
  for (const t of tables) {
    const refs = t.foreign_keys
      .map((fk) => key({ schema: fk.ref_schema, name: fk.ref_table }))
      .filter((r) => r !== key(t) && inBlock.has(r));
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
  for (const t of tables) {
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

  const at: Record<string, Point> = {};
  let height = 0;
  columns.forEach((column, c) => {
    let y = 0;
    for (const k of column) {
      at[k] = [c * (WIDTH + GAP_X), y];
      y += tableHeight(byKey.get(k)!) + GAP_Y;
    }
    height = Math.max(height, y - GAP_Y);
  });
  return { at, width: columns.length * (WIDTH + GAP_X) - GAP_X, height };
}

export type Frame = { schema: string; x: number; y: number; width: number; height: number };

/** A box around each schema's tables, wherever they were dragged. None with one schema. */
export function schemaFrames(schema: Schema, positions: Record<string, Point>): Frame[] {
  const groups = schemaGroups(schema);
  if (groups.size < 2) return [];
  const frames: Frame[] = [];
  for (const [name, tables] of groups) {
    const boxes = tables.flatMap((t) => {
      const p = positions[tableKey(schema.engine, t)];
      return p ? [[p[0], p[1], p[0] + WIDTH, p[1] + tableHeight(t)]] : [];
    });
    if (boxes.length === 0) continue;
    const x = Math.min(...boxes.map((b) => b[0])) - FRAME_PAD;
    const y = Math.min(...boxes.map((b) => b[1])) - FRAME_PAD - FRAME_LABEL;
    const right = Math.max(...boxes.map((b) => b[2])) + FRAME_PAD;
    const bottom = Math.max(...boxes.map((b) => b[3])) + FRAME_PAD;
    frames.push({ schema: name, x, y, width: right - x, height: bottom - y });
  }
  return frames;
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
