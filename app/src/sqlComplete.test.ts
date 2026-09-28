import { describe, expect, it } from "vitest";
import type { CompletionTable } from "./api";
import { complete, ident } from "./sqlComplete";

const tables: CompletionTable[] = [
  { schema: "public", name: "album", columns: ["album_id", "title", "artist_id"] },
  { schema: "public", name: "artist", columns: ["artist_id", "name"] },
  { schema: "billing", name: "invoice", columns: ["invoice_id", "total"] },
  { schema: "billing", name: "Weird Name", columns: ["Mixed"] },
];

/** `|` marks the cursor. */
function at(sql: string, activeSchema: string | null = null) {
  const pos = sql.indexOf("|");
  return complete(sql.replace("|", ""), pos, tables, { engine: "postgres", activeSchema });
}
const labels = (sql: string) => at(sql)?.options.filter((o) => o.type !== "variable").map((o) => o.label) ?? null;

describe("columns", () => {
  it("come from the table named after the cursor", () => {
    expect(labels("select | from album")).toEqual(["album_id", "title", "artist_id"]);
    expect(labels("select title, a| from album where title = 'x'")).toEqual(["album_id", "title", "artist_id"]);
  });

  it("cover every joined table, and say which", () => {
    const r = at("select | from album a join artist r on r.artist_id = a.artist_id")!;
    expect(r.options.filter((o) => o.type === "column").map((o) => o.displayLabel ?? `${o.label} (${o.detail})`)).toEqual([
      "album_id (a)", "title (a)", "a.artist_id", "r.artist_id", "name (r)",
    ]);
  });

  it("qualify a column two tables share, so it runs", () => {
    const r = at("select | from album a join artist r using (artist_id)")!;
    expect(r.options.filter((o) => o.label === "artist_id").map((o) => o.apply)).toEqual(["a.artist_id", "r.artist_id"]);
    expect(r.options.find((o) => o.label === "title")!.apply).toBe("title");
  });

  it("offer the tables' aliases as high as their columns", () => {
    const r = at("select | from album a join artist r on true")!;
    const alias = r.options.find((o) => o.label === "r")!;
    expect(alias).toMatchObject({ type: "variable", detail: "alias of artist" });
    expect(alias.boost).toBe(r.options.find((o) => o.label === "name")!.boost);
  });

  it("offer the select list's aliases in ORDER BY, GROUP BY and HAVING", () => {
    const sql = "select g.name as genre, sum(il.title) as sold from album il join artist g on true group by 1 order by |";
    expect(at(sql)!.options.slice(0, 2).map((o) => o.label)).toEqual(["genre", "sold"]);
    expect(at("select title as t from album where |")!.options.some((o) => o.detail === "select alias")).toBe(false);
  });

  it("narrow to one table after its alias or name", () => {
    expect(labels("select r.| from album a join artist r on true")).toEqual(["artist_id", "name"]);
    expect(labels("select album.ti| from album")).toEqual(["album_id", "title", "artist_id"]);
  });

  it("follow WHERE, ON, ORDER BY and SET", () => {
    expect(labels("select * from artist where |")).toEqual(["artist_id", "name"]);
    expect(labels("select * from artist order by |")).toEqual(["artist_id", "name"]);
    expect(labels("update artist set |")).toEqual(["artist_id", "name"]);
  });

  it("list the insert target's columns inside its brackets", () => {
    expect(labels("insert into artist (|")).toEqual(["artist_id", "name"]);
  });

  it("stay within the statement under the cursor", () => {
    expect(labels("select * from album;\nselect | from artist")).toEqual(["artist_id", "name"]);
  });

  it("quote what needs quoting", () => {
    expect(at('select | from billing."Weird Name"')!.options[0].apply).toBe('"Mixed"');
  });
});

describe("tables", () => {
  it("add the schema when it isn't the default one", () => {
    const r = at("select * from inv|")!;
    expect(r.from).toBe("select * from ".length);
    expect(r.options.find((o) => o.label === "invoice")!.apply).toBe("billing.invoice");
    expect(r.options.find((o) => o.label === "album")!.apply).toBe("album");
    expect(r.options.find((o) => o.label === "Weird Name")!.apply).toBe('billing."Weird Name"');
  });

  it("after a schema, are that schema's only, without repeating it", () => {
    expect(at("select * from billing.|")!.options.map((o) => o.apply)).toEqual(["invoice", '"Weird Name"']);
  });

  it("follow JOIN and FROM lists", () => {
    expect(labels("select * from album a join |")).toContain("artist");
    expect(labels("select * from album a, |")).toContain("invoice");
  });

  it("put the picked schema first", () => {
    const boost = (name: string, schema: string | null) => at("select * from |", schema)!.options.find((o) => o.label === name)!.boost!;
    expect(boost("invoice", "billing")).toBeGreaterThan(boost("album", "billing"));
    expect(boost("album", null)).toBeGreaterThan(boost("invoice", null));
  });
});

describe("nothing specific", () => {
  it("while naming an alias, in strings and comments, or without a table", () => {
    expect(at("select * from album a|")).toBeNull();
    expect(at("select * from album where title = 'ti|'")).toBeNull();
    expect(at("select * from album -- ti|")).toBeNull();
    expect(at("select |")).toBeNull();
  });
});

describe("ident", () => {
  it("quotes capitals, spaces and keywords for Postgres", () => {
    expect(ident("album", "postgres")).toBe("album");
    expect(ident("Album", "postgres")).toBe('"Album"');
    expect(ident("order", "postgres")).toBe('"order"');
    expect(ident("Album", "sqlite")).toBe("Album");
  });
});
