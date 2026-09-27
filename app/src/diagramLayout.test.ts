import { describe, expect, it } from "vitest";
import type { Schema, SchemaTable } from "./api";
import { WIDTH, edgePath, layOut, tableKey } from "./diagramLayout";

function table(name: string, refs: string[] = [], schema = "public"): SchemaTable {
  return {
    schema,
    name,
    columns: [{ name: "id", data_type: "int", nullable: false, default: null, primary_key: true }],
    indexes: [],
    foreign_keys: refs.map((r) => ({ name: null, columns: ["id"], ref_schema: "public", ref_table: r, ref_columns: ["id"] })),
  };
}

const schema = (tables: SchemaTable[]): Schema => ({ engine: "postgres", database: "db", tables, enums: [] });

describe("tableKey", () => {
  it("leaves out the default schema, as DBML does", () => {
    expect(tableKey("postgres", { schema: "public", name: "album" })).toBe("album");
    expect(tableKey("postgres", { schema: "perf", name: "big" })).toBe("perf.big");
    expect(tableKey("sqlite", { schema: "main", name: "Album" })).toBe("Album");
  });
});

describe("layOut", () => {
  it("puts a table right of what it references", () => {
    const p = layOut(schema([table("track", ["album"]), table("album", ["artist"]), table("artist")]), {});
    expect(p.artist[0]).toBeLessThan(p.album[0]);
    expect(p.album[0]).toBeLessThan(p.track[0]);
  });

  it("keeps saved positions and places new tables below them", () => {
    const p = layOut(schema([table("a"), table("b", ["a"])]), { a: [500, 400], gone: [0, 0] });
    expect(p.a).toEqual([500, 400]);
    expect(p.b[1]).toBeGreaterThan(400);
    expect(p).not.toHaveProperty("gone");
  });

  it("survives reference cycles and self-references", () => {
    const p = layOut(schema([table("a", ["b"]), table("b", ["a"]), table("employee", ["employee"])]), {});
    expect(Object.keys(p).sort()).toEqual(["a", "b", "employee"]);
  });

  it("wraps a tall column instead of growing forever", () => {
    const many = Array.from({ length: 60 }, (_, i) => table(`t${i}`));
    const xs = new Set(Object.values(layOut(schema(many), {})).map(([x]) => x));
    expect(xs.size).toBeGreaterThan(1);
  });
});

describe("edgePath", () => {
  it("runs from the side facing the other table", () => {
    expect(edgePath([400, 0], 0, [0, 0], 0).startsWith("M400,")).toBe(true);
    expect(edgePath([0, 0], 0, [400, 0], 0).startsWith(`M${WIDTH},`)).toBe(true);
  });
});
