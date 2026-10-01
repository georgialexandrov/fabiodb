// Column widths someone sets by hand: dragged, or fitted to the text with a double-click.

export const MIN_COLUMN_WIDTH = 40;
export const MAX_COLUMN_WIDTH = 1200;
/** Cell padding (10px a side) and its right border. */
const CELL_CHROME = 21;

export function clampColumnWidth(width: number) {
  return Math.round(Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, width)));
}

/** Wide enough for the header and every value given, as the cells show them. */
export function fitColumnWidth(header: number, values: (string | null)[], measure: (text: string) => number) {
  const widest = values.reduce((m, v) => Math.max(m, measure(v === null ? "NULL" : preview(v))), 0);
  return clampColumnWidth(Math.max(header, widest + CELL_CHROME));
}

/** First line only, capped — the full value lives in the detail panel. */
export function preview(value: string) {
  const line = value.length > 300 ? value.slice(0, 300) : value;
  const nl = line.indexOf("\n");
  return nl === -1 ? line : `${line.slice(0, nl)} ⏎`;
}
