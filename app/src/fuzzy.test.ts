import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzyScore", () => {
  it("needs every character, in order", () => {
    expect(fuzzyScore("trk", "track")).not.toBeNull();
    expect(fuzzyScore("tkr", "track")).toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
  });

  it("ignores case and spaces in the query", () => {
    expect(fuzzyScore("New Q", "new query")).not.toBeNull();
  });
});

describe("fuzzyFilter", () => {
  const tables = ["playlist_track", "track", "invoice_line", "Open table: public.track"];

  it("ranks word starts and tight runs first", () => {
    expect(fuzzyFilter(tables, "track", (t) => t)[0]).toBe("track");
    expect(fuzzyFilter(tables, "il", (t) => t)[0]).toBe("invoice_line");
  });

  it("keeps everything, in order, for an empty query", () => {
    expect(fuzzyFilter(tables, " ", (t) => t)).toEqual(tables);
  });
});
