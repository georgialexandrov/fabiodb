import { describe, expect, it } from "vitest";
import { allowed, columnKinds, niceTicks, pick, toTime, type Kind } from "./chartSpec";

const kindsOf = (columns: string[], rows: (string | null)[][]) => columnKinds(columns, rows);
const formOf = (columns: string[], rows: (string | null)[][]) => pick(columns, kindsOf(columns, rows), rows).form;

describe("columnKinds", () => {
  it("reads numbers, dates and timestamps from the text values", () => {
    const rows = [
      ["Rock", "826.65", "2021-01-01", "2021-01-01 00:00:00+00", "-3e2"],
      ["Jazz", "79", "2021-02-01", "2021-02-01 10:30:00.5+02:00", null],
    ];
    expect(kindsOf(["genre", "revenue", "month", "at", "x"], rows)).toEqual(["text", "number", "time", "time", "number"]);
  });

  it("treats numeric ids as keys, not measures", () => {
    expect(kindsOf(["customer_id", "id", "total"], [["1", "2", "3.5"]])).toEqual(["key", "key", "number"]);
  });

  it("calls an all-NULL column text", () => {
    expect(kindsOf(["x"], [[null], [null]])).toEqual(["text"]);
  });

  it("calls a column text as soon as one value isn't a number", () => {
    expect(kindsOf(["zip"], [["1000"], ["10A"]])).toEqual(["text"]);
  });
});

describe("pick", () => {
  it("bars a number per category", () => {
    expect(formOf(["genre", "revenue"], [["Rock", "826.65"], ["Latin", "382.14"]])).toBe("bar");
  });

  it("lines a number over time", () => {
    const spec = pick(["month", "revenue"], ["time", "number"], [["2021-01-01", "1"], ["2021-02-01", "2"]]);
    expect(spec).toMatchObject({ form: "line", x: 0, y: 1, series: null });
  });

  it("draws a line per text value when there is one", () => {
    const spec = pick(["quarter", "country", "revenue"], ["time", "text", "number"], [["2021-01-01", "USA", "1"], ["2021-01-01", "France", "2"]]);
    expect(spec).toMatchObject({ form: "line", x: 0, y: 2, series: 1 });
  });

  it("lines a number against an increasing whole number, like a year", () => {
    const rows = [["2021", "10.5"], ["2022", "12"], ["2023", "9"]];
    expect(pick(["year", "revenue"], kindsOf(["year", "revenue"], rows), rows)).toMatchObject({ form: "line", x: 0, y: 1 });
  });

  it("scatters two numbers over many rows, naming points by the text column", () => {
    const rows = Array.from({ length: 60 }, (_, i) => [`t${i}`, String(i * 1.5), String(i * 2 + 1)]);
    rows.reverse();
    expect(pick(["name", "minutes", "mb"], kindsOf(["name", "minutes", "mb"], rows), rows)).toMatchObject({ form: "scatter", x: 1, y: 2, label: 0 });
  });

  it("bars a few categories even with two numbers", () => {
    expect(formOf(["genre", "tracks", "revenue"], [["Rock", "10", "5.5"], ["Jazz", "3", "1"]])).toBe("bar");
  });

  it("bars by a key column", () => {
    expect(formOf(["customer_id", "total"], [["5", "40"], ["2", "38"]])).toBe("bar");
  });

  it("shows a single value as a number", () => {
    expect(formOf(["revenue"], [["2328.60"]])).toBe("value");
  });

  it("has nothing to chart without a number", () => {
    const spec = pick(["name"], ["text"], [["AC/DC"]]);
    expect(spec.form).toBe("none");
    expect(spec.why).toMatch(/number/);
  });

  it("has nothing to compare in one row", () => {
    expect(formOf(["genre", "revenue"], [["Rock", "1"]])).toBe("none");
  });
});

describe("allowed", () => {
  const k: Kind[] = ["text", "number"];
  it("needs two numbers for a scatter", () => {
    expect(allowed("scatter", k)).toBe(false);
    expect(allowed("scatter", ["number", "number"])).toBe(true);
  });
  it("needs a number and something to put it against for a bar or line", () => {
    expect(allowed("bar", k)).toBe(true);
    expect(allowed("line", ["text"])).toBe(false);
    expect(allowed("bar", ["number"])).toBe(false);
  });
});

describe("niceTicks", () => {
  it("steps by 1, 2, 2.5 or 5 and covers the max", () => {
    expect(niceTicks(826.65)).toEqual([0, 200, 400, 600, 800, 1000]);
    expect(niceTicks(23)).toEqual([0, 5, 10, 15, 20, 25]);
  });
  it("goes below zero for negative values", () => {
    expect(niceTicks(10, -4)).toEqual([-5, 0, 5, 10]);
  });
  it("copes with zero", () => {
    expect(niceTicks(0)).toEqual([0, 1]);
  });
});

describe("toTime", () => {
  it("parses Postgres timestamptz text", () => {
    expect(toTime("2021-01-01 00:00:00+00")).toBe(Date.UTC(2021, 0, 1));
    expect(toTime("2021-01-01 02:00:00+02")).toBe(Date.UTC(2021, 0, 1));
  });
  it("reads a date as UTC midnight", () => {
    expect(toTime("2021-03-01")).toBe(Date.UTC(2021, 2, 1));
  });
});
