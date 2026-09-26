import { describe, expect, it } from "vitest";
import { splitStatements, statementAt } from "./statements";

const texts = (sql: string) => splitStatements(sql).map((s) => s.text);

describe("splitStatements", () => {
  it("splits on semicolons and trims", () => {
    expect(texts("select 1;  select 2 ;\n\nselect 3")).toEqual(["select 1", "select 2", "select 3"]);
  });

  it("ignores semicolons inside strings, identifiers and comments", () => {
    const sql = `select 'a;b', "we;ird" from t -- x;y
      /* z; */ where 1 = 1; select 2`;
    expect(texts(sql)).toHaveLength(2);
    expect(texts(sql)[0]).toContain("where 1 = 1");
  });

  it("handles doubled quotes", () => {
    expect(texts("select 'it''s; fine'; select 2")).toEqual(["select 'it''s; fine'", "select 2"]);
  });

  it("ignores semicolons inside dollar-quoted bodies", () => {
    const sql = "create function f() returns int as $body$ begin; return 1; end $body$ language plpgsql; select 2";
    expect(texts(sql)).toHaveLength(2);
    expect(texts("select $$a;b$$; select 2")).toEqual(["select $$a;b$$", "select 2"]);
  });

  it("drops empty statements", () => {
    expect(texts(";;  ;\n select 1;;")).toEqual(["select 1"]);
  });

  it("reports offsets into the original text", () => {
    const sql = "  select 1;\n  select 2";
    const [a, b] = splitStatements(sql);
    expect(sql.slice(a.from, a.to)).toBe("select 1");
    expect(sql.slice(b.from, b.to)).toBe("select 2");
  });

  it("does not hang on unterminated quotes or comments", () => {
    expect(texts("select 'oops; select 2")).toEqual(["select 'oops; select 2"]);
    expect(texts("select 1 /* never closed; ")).toHaveLength(1);
  });
});

describe("statementAt", () => {
  const sql = "select 1;\nselect 2;\n\nselect 3";
  const stmts = splitStatements(sql);

  it("finds the statement under the cursor", () => {
    expect(statementAt(stmts, sql.indexOf("2"))?.text).toBe("select 2");
  });

  it("uses the statement just before the cursor when it sits in the gap", () => {
    expect(statementAt(stmts, sql.indexOf("\n\n") + 1)?.text).toBe("select 2");
  });

  it("treats a cursor right after the semicolon as that statement", () => {
    expect(statementAt(stmts, sql.indexOf(";") + 1)?.text).toBe("select 1");
  });

  it("is undefined for empty input", () => {
    expect(statementAt([], 0)).toBeUndefined();
  });
});
