import { describe, expect, it } from "vitest";
import { clampColumnWidth, fitColumnWidth, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH } from "./columnWidth";

// One pixel per character keeps the arithmetic readable.
const measure = (text: string) => text.length;

describe("fitColumnWidth", () => {
  it("fits the longest value, plus the cell's padding", () => {
    expect(fitColumnWidth(100, ["a".repeat(150), "short"], measure)).toBe(150 + 21);
  });

  it("is never narrower than the header", () => {
    expect(fitColumnWidth(120, ["x"], measure)).toBe(120);
  });

  it("shows NULL as NULL", () => {
    expect(fitColumnWidth(0, [null], measure)).toBe(Math.max(MIN_COLUMN_WIDTH, 4 + 21));
  });

  it("measures the first line only, as the cell shows it", () => {
    expect(fitColumnWidth(0, ["a".repeat(80) + "\n" + "b".repeat(500)], measure)).toBe(80 + 2 + 21);
  });

  it("stops at a sane maximum", () => {
    expect(fitColumnWidth(0, ["a".repeat(5000)], (t) => t.length * 7.4)).toBe(MAX_COLUMN_WIDTH);
  });
});

describe("clampColumnWidth", () => {
  it("keeps a dragged width between the limits, in whole pixels", () => {
    expect(clampColumnWidth(3)).toBe(MIN_COLUMN_WIDTH);
    expect(clampColumnWidth(99999)).toBe(MAX_COLUMN_WIDTH);
    expect(clampColumnWidth(123.6)).toBe(124);
  });
});
