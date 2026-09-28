import type { Rows } from "./api";

/**
 * Charts on query results: which chart a result gets, from its columns.
 * Results arrive as text without types (SQLite has none to give), so a
 * column's kind is read from its values.
 */
export type Kind = "number" | "time" | "key" | "text";
export type Form = "line" | "bar" | "pie" | "histogram" | "scatter" | "value" | "none";
export type Spec = {
  form: Form;
  x: number | null;
  y: number | null;
  /** Line: one line per value of this column. */
  series: number | null;
  /** Scatter: names each point. */
  label: number | null;
  why: string;
};

const NUMBER = /^-?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;
const TIME = /^\d{4}-\d{2}-\d{2}([ T]\d{2}:\d{2}(:\d{2}(\.\d+)?)?)?(Z|[+-]\d{2}(:?\d{2})?)?$/;
/** Enough rows to judge a column by; the rest are assumed to agree. */
const SAMPLE = 500;

export function columnKinds(columns: string[], rows: Rows): Kind[] {
  return columns.map((name, i) => {
    let kind: Kind | null = null;
    for (let r = 0, seen = 0; r < rows.length && seen < SAMPLE; r++) {
      const v = rows[r][i];
      if (v === null) continue;
      seen++;
      const k = NUMBER.test(v) ? "number" : TIME.test(v) ? "time" : "text";
      if (kind && k !== kind) return "text";
      kind = k;
    }
    if (kind === "number" && /^(id|.*_id)$/i.test(name)) return "key";
    return kind ?? "text";
  });
}

/** Milliseconds since the epoch. A date, or a timestamp without a zone, is read as UTC. */
export function toTime(v: string): number {
  if (v.length === 10) return Date.parse(`${v}T00:00:00Z`);
  let s = v.replace(" ", "T");
  const zone = /([+-]\d{2})(:?(\d{2}))?$/.exec(s);
  if (zone && s.indexOf("T") < zone.index) s = s.slice(0, zone.index) + `${zone[1]}:${zone[3] ?? "00"}`;
  else if (!s.endsWith("Z")) s += "Z";
  return Date.parse(s);
}

const indexes = (kinds: Kind[], ...want: Kind[]) => kinds.flatMap((k, i) => (want.includes(k) ? [i] : []));

/** Whole numbers going up row by row: a year, an hour, a bucket. */
function increasing(rows: Rows, col: number): boolean {
  let last = -Infinity;
  for (const r of rows) {
    const v = r[col];
    if (v === null || !/^-?\d+$/.test(v) || +v <= last) return false;
    last = +v;
  }
  return rows.length > 1;
}

const blank = { x: null, y: null, series: null, label: null };

export function pick(columns: string[], kinds: Kind[], rows: Rows): Spec {
  const nums = indexes(kinds, "number");
  const times = indexes(kinds, "time");
  const texts = indexes(kinds, "text");
  const categories = indexes(kinds, "text", "key");
  const name = (i: number) => columns[i];

  if (!nums.length) return { ...blank, form: "none", why: "Nothing to chart: there's no number column." };
  if (columns.length === 1 && rows.length === 1) return { ...blank, form: "value", y: nums[0], why: "" };
  if (rows.length < 2) return { ...blank, form: "none", why: "Nothing to compare in one row." };

  if (times.length)
    return {
      ...blank,
      form: "line",
      x: times[0],
      y: nums[0],
      series: texts[0] ?? null,
      why: `${name(times[0])} is a time, so it runs along the bottom` + (texts.length ? `, a line per ${name(texts[0])}.` : "."),
    };
  if (nums.length >= 2 && nums[0] === 0 && increasing(rows, 0))
    return { ...blank, form: "line", x: 0, y: nums[1], why: `${name(0)} goes up row by row, so it runs along the bottom.` };
  if (nums.length >= 2 && (rows.length > 40 || !categories.length))
    return {
      ...blank,
      form: "scatter",
      x: nums[0],
      y: nums[1],
      label: texts[0] ?? null,
      why: `Two numbers: ${name(nums[1])} against ${name(nums[0])}.`,
    };
  if (nums.length === 1 && !categories.length && rows.length >= 10)
    return { ...blank, form: "histogram", y: nums[0], why: `How ${name(nums[0])} is spread: rows per range of values.` };
  if (categories.length)
    return { ...blank, form: "bar", x: categories[0], y: nums[0], why: `One ${name(nums[0])} per ${name(categories[0])}, in the order the query returned them.` };
  return { ...blank, form: "none", why: "One number per row and nothing to put it against." };
}

export function allowed(form: Form, kinds: Kind[]): boolean {
  const nums = indexes(kinds, "number").length;
  if (form === "scatter") return nums >= 2;
  if (form === "histogram") return nums >= 1;
  if (form === "line" || form === "bar" || form === "pie") return nums >= 1 && kinds.length >= 2;
  return true;
}

/** Switching form by hand keeps the chosen columns where they still fit. */
export function reshape(spec: Spec, form: Form, kinds: Kind[]): Spec {
  const nums = indexes(kinds, "number");
  const why = "";
  if (form === "scatter") return { ...blank, form, x: nums[0], y: nums[1], label: indexes(kinds, "text")[0] ?? null, why };
  if (form === "histogram") return { ...blank, form, y: spec.y !== null && kinds[spec.y] === "number" ? spec.y : nums[0], why };
  const other = kinds.findIndex((k) => k !== "number");
  const x = spec.x !== null && spec.x !== spec.y ? spec.x : other >= 0 ? other : nums[0];
  const y = spec.y !== null && spec.y !== x && kinds[spec.y] === "number" ? spec.y : (nums.find((i) => i !== x) ?? nums[0]);
  return { ...blank, form, x, y, series: form === "line" ? spec.series : null, why };
}

/** Axis ticks covering min (never above 0) to max, stepping by 1, 2, 2.5 or 5 times a power of ten. */
export function niceTicks(max: number, min = 0, count = 5): number[] {
  const lo = Math.min(0, min);
  const hi = Math.max(0, max);
  if (!(hi - lo > 0)) return [0, 1];
  const raw = (hi - lo) / count;
  const p = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= raw)!;
  const out: number[] = [];
  for (let i = Math.floor(lo / step); out.length === 0 || out[out.length - 1] < hi; i++) out.push(+(i * step).toPrecision(12));
  return out;
}

/** A pie reads at a glance up to this many slices; past it the smallest fold into "Other". */
export const PIE_SLICES = 6;

export type Slice = { label: string; value: number; other?: boolean };

/**
 * Slices largest first, the tail folded into one "Other" so there are at most
 * PIE_SLICES. Null if a value is negative: parts of a whole can't be.
 */
export function pieSlices(rows: Rows, x: number, y: number): Slice[] | null {
  const totals = new Map<string, number>();
  for (const r of rows) {
    if (r[y] === null) continue;
    const v = +r[y]!;
    if (v < 0) return null;
    const k = r[x] ?? "NULL";
    totals.set(k, (totals.get(k) ?? 0) + v);
  }
  const all = [...totals].map(([label, value]) => ({ label, value })).sort((a, b) => b.value - a.value);
  if (all.length <= PIE_SLICES) return all;
  const rest = all.slice(PIE_SLICES - 1);
  return [...all.slice(0, PIE_SLICES - 1), { label: `Other (${rest.length})`, value: rest.reduce((a, b) => a + b.value, 0), other: true }];
}

export type Bin = { from: number; to: number; count: number };

/** Equal ranges on round edges, about √n of them (5–40); each counts from ≤ v < to, the last includes its end. */
export function histogram(values: number[]): Bin[] {
  if (!values.length) return [];
  let lo = Infinity, hi = -Infinity;
  for (const v of values) (lo = Math.min(lo, v)), (hi = Math.max(hi, v));
  if (lo === hi) return [{ from: lo, to: hi, count: values.length }];
  const want = Math.min(40, Math.max(5, Math.round(Math.sqrt(values.length))));
  const raw = (hi - lo) / want;
  const p = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * p).find((s) => s >= raw)!;
  const start = Math.floor(lo / step) * step;
  const n = Math.max(1, Math.ceil((hi - start) / step - 1e-9));
  const bins: Bin[] = Array.from({ length: n }, (_, i) => ({ from: +(start + i * step).toPrecision(12), to: +(start + (i + 1) * step).toPrecision(12), count: 0 }));
  for (const v of values) bins[Math.min(n - 1, Math.floor((v - start) / step))].count++;
  return bins;
}
