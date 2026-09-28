import type { CompletionTable } from "./api";
import { splitStatements, statementAt } from "./statements";

export type Engine = "postgres" | "sqlite";

export type Suggestion = {
  label: string;
  /** What gets inserted, when it differs from the label (quoting, a schema). */
  apply?: string;
  type: "column" | "table" | "namespace" | "variable";
  detail?: string;
  boost?: number;
};

/** Replace `from`–`to` with one of `options`. */
export type Suggestions = { from: number; to: number; options: Suggestion[] };

export type CompleteOptions = {
  engine: Engine;
  /** The schema picked in the sidebar: its tables come first. */
  activeSchema?: string | null;
};

type Token = { kind: "word" | "quoted" | "string" | "comment" | "punct" | "other"; text: string; value: string; from: number; to: number };

/** Tables are named after these; columns follow the rest. */
const TABLE_CLAUSES = new Set(["from", "join", "update", "into", "table"]);
const CLAUSES = new Set([
  ...TABLE_CLAUSES,
  "select", "on", "where", "and", "or", "by", "having", "set", "returning", "using",
  "values", "when", "then", "else", "case", "not", "distinct", "limit", "offset",
]);
/** Words that end a table reference instead of naming or aliasing it. */
const RESERVED = new Set([
  ...CLAUSES,
  "all", "any", "as", "asc", "both", "check", "column", "constraint", "create", "cross", "default", "desc",
  "end", "except", "false", "fetch", "for", "foreign", "full", "grant", "group", "in", "inner", "intersect",
  "is", "lateral", "left", "natural", "null", "only", "order", "outer", "primary", "references", "right",
  "true", "union", "unique", "user", "window", "with",
]);

export function defaultSchema(engine: Engine): string {
  return engine === "postgres" ? "public" : "main";
}

/** An identifier as SQL needs it written: quoted when folding or a keyword would change it. */
export function ident(name: string, engine: Engine): string {
  const plain = engine === "postgres" ? /^[a-z_][a-z0-9_$]*$/ : /^[A-Za-z_][A-Za-z0-9_]*$/;
  return plain.test(name) && !RESERVED.has(name.toLowerCase()) ? name : `"${name.split('"').join('""')}"`;
}

/**
 * What fits at `pos`: after FROM/JOIN a table (with its schema when it isn't
 * the default one), elsewhere the columns of the tables the statement names —
 * before or after the cursor. `alias.` narrows to that table. Not a parser:
 * subqueries share one scope. Null when nothing specific fits (keywords do).
 */
export function complete(text: string, pos: number, tables: CompletionTable[], opts: CompleteOptions): Suggestions | null {
  const stmt = statementAt(splitStatements(text), pos);
  const start = stmt && pos >= stmt.from ? stmt.from : pos;
  const end = stmt && pos <= stmt.to ? stmt.to : pos;
  const tokens = tokenize(text.slice(start, end), start);
  if (tokens.some((t) => (t.kind === "string" || t.kind === "comment") && t.from < pos && pos <= t.to)) return null;

  // The word being typed, an opening quote included.
  let from = pos;
  while (from > 0 && /[\w$\u0080-￿]/.test(text[from - 1])) from--;
  let to = pos;
  if (text[from - 1] === '"') {
    from--;
    if (text[pos] === '"') to = pos + 1; // closeBrackets' quote
  }
  let qualifier: string | null = null;
  let before = from;
  if (text[from - 1] === ".") {
    const q = tokens.filter((t) => t.to === from - 1 && (t.kind === "word" || t.kind === "quoted")).pop();
    if (!q) return null;
    qualifier = q.value;
    before = q.from;
  }
  const prev = tokens.filter((t) => t.to <= before && t.kind !== "comment");
  const { keyword, inParens } = clauseOf(prev);
  const last = prev[prev.length - 1];
  const refs = tableRefs(tokens.filter((t) => t.kind !== "comment"));
  const result = (options: Suggestion[]) => (options.length ? { from, to, options } : null);

  const schemas = [...new Set(tables.map((t) => t.schema))];
  const findSchema = (name: string) => schemas.find((s) => same(s, name));

  if (keyword && TABLE_CLAUSES.has(keyword) && !inParens) {
    const afterKeyword = last && (same(last.value, keyword) || last.text === "," || ["only", "lateral"].includes(last.value.toLowerCase()));
    if (!afterKeyword) return null; // an alias, or the next keyword
    if (qualifier !== null) {
      const schema = findSchema(qualifier);
      return schema ? result(tableOptions(tables.filter((t) => t.schema === schema), opts, false)) : null;
    }
    const namespaces: Suggestion[] = schemas
      .filter((s) => s !== defaultSchema(opts.engine))
      .map((s) => ({ label: s, apply: `${ident(s, opts.engine)}.`, type: "namespace", boost: -1 }));
    return result([...tableOptions(tables, opts, true), ...namespaces]);
  }
  if (keyword === "values" || keyword === "limit" || keyword === "offset") return null;

  // insert into t (…: that table's columns only.
  let scope = refs.map((r) => ({ ref: r, table: resolve(r, tables, opts) })).filter((s) => s.table) as { ref: Ref; table: CompletionTable }[];
  if (keyword === "into" && inParens) {
    const into = intoRef(prev);
    scope = scope.filter((s) => into && same(s.ref.name, into.name) && same(s.ref.schema ?? "", into.schema ?? ""));
  }

  if (qualifier !== null) {
    const q = qualifier;
    const hit =
      scope.find((s) => s.ref.alias && same(s.ref.alias, q)) ?? scope.find((s) => !s.ref.alias && same(s.ref.name, q));
    if (hit) return result(columnOptions([hit], opts, false));
    const schema = findSchema(q);
    if (schema) return result(tableOptions(tables.filter((t) => t.schema === schema), opts, false));
    const table = tables.find((t) => same(t.name, q));
    return table ? result(columnOptions([{ ref: { name: table.name }, table }], opts, false)) : null;
  }
  if (scope.length === 0) return null;
  const names: Suggestion[] = scope.map((s) => ({
    label: s.ref.alias ?? s.table.name,
    apply: ident(s.ref.alias ?? s.table.name, opts.engine),
    type: "variable",
    detail: s.ref.alias ? s.table.name : s.table.schema,
    boost: -1,
  }));
  return result([...columnOptions(scope, opts, scope.length > 1), ...names]);
}

function tableOptions(tables: CompletionTable[], opts: CompleteOptions, withSchema: boolean): Suggestion[] {
  const home = defaultSchema(opts.engine);
  return tables.map((t) => ({
    label: t.name,
    apply: withSchema && t.schema !== home ? `${ident(t.schema, opts.engine)}.${ident(t.name, opts.engine)}` : ident(t.name, opts.engine),
    type: "table",
    detail: t.schema,
    boost: opts.activeSchema && t.schema === opts.activeSchema ? 2 : t.schema === home ? 1 : 0,
  }));
}

function columnOptions(scope: { ref: Ref; table: CompletionTable }[], opts: CompleteOptions, sayWhich: boolean): Suggestion[] {
  return scope.flatMap(({ ref, table }) =>
    table.columns.map((c): Suggestion => ({
      label: c,
      apply: ident(c, opts.engine),
      type: "column",
      detail: sayWhich ? (ref.alias ?? table.name) : undefined,
      boost: 1,
    })),
  );
}

type Ref = { schema?: string; name: string; alias?: string };

/** Every `[schema.]table [as] [alias]` after FROM (and its commas), JOIN, UPDATE and INTO. */
function tableRefs(tokens: Token[]): Ref[] {
  const refs: Ref[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const kw = tokens[i].kind === "word" ? tokens[i].value.toLowerCase() : "";
    if (!TABLE_CLAUSES.has(kw)) continue;
    let j = i + 1;
    for (;;) {
      while (tokens[j]?.kind === "word" && ["only", "lateral"].includes(tokens[j].value.toLowerCase())) j++;
      const read = readRef(tokens, j);
      if (!read) break;
      refs.push(read.ref);
      j = read.next;
      if (kw !== "from" || tokens[j]?.text !== ",") break;
      j++;
    }
  }
  return refs;
}

function readRef(tokens: Token[], j: number): { ref: Ref; next: number } | null {
  if (!isName(tokens[j])) return null;
  const ref: Ref = { name: tokens[j].value };
  let k = j + 1;
  if (tokens[k]?.text === "." && isName(tokens[k + 1])) {
    ref.schema = ref.name;
    ref.name = tokens[k + 1].value;
    k += 2;
  }
  if (tokens[k]?.kind === "word" && tokens[k].value.toLowerCase() === "as") k++;
  if (isName(tokens[k])) ref.alias = tokens[k++].value;
  return { ref, next: k };
}

function isName(t: Token | undefined): t is Token {
  return !!t && (t.kind === "quoted" || (t.kind === "word" && !RESERVED.has(t.value.toLowerCase())));
}

/** The table of the nearest `into t (`, as tableRefs read it. */
function intoRef(prev: Token[]): Ref | undefined {
  for (let i = prev.length - 1; i >= 0; i--) {
    if (prev[i].kind === "word" && prev[i].value.toLowerCase() === "into") return tableRefs(prev.slice(i))[0];
  }
}

/** The clause keyword the cursor is in, and whether a bracket opened since it. */
function clauseOf(prev: Token[]): { keyword: string | null; inParens: boolean } {
  let depth = 0;
  let inParens = false;
  for (let i = prev.length - 1; i >= 0; i--) {
    const t = prev[i];
    if (t.text === ")") depth++;
    else if (t.text === "(") {
      if (depth > 0) depth--;
      else inParens = true;
    } else if (depth === 0 && t.kind === "word" && CLAUSES.has(t.value.toLowerCase())) {
      // ORDER BY / GROUP BY, NOT NULL…: the column context they share.
      return { keyword: t.value.toLowerCase(), inParens };
    }
  }
  return { keyword: null, inParens };
}

function resolve(ref: Ref, tables: CompletionTable[], opts: CompleteOptions): CompletionTable | undefined {
  const named = tables.filter((t) => same(t.name, ref.name) && (!ref.schema || same(t.schema, ref.schema)));
  if (named.length <= 1 || ref.schema) return named[0];
  return named.find((t) => t.schema === defaultSchema(opts.engine)) ?? named.find((t) => t.schema === opts.activeSchema) ?? named[0];
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

function tokenize(sql: string, base: number): Token[] {
  const out: Token[] = [];
  const n = sql.length;
  const push = (kind: Token["kind"], i: number, j: number, value = sql.slice(i, j)) =>
    out.push({ kind, text: sql.slice(i, j), value, from: base + i, to: base + j });
  let i = 0;
  while (i < n) {
    const c = sql[i];
    let j = i + 1;
    if (/\s/.test(c)) {
      i++;
      continue;
    } else if (c === "-" && sql[i + 1] === "-") {
      const e = sql.indexOf("\n", i);
      j = e === -1 ? n : e;
      push("comment", i, j);
    } else if (c === "/" && sql[i + 1] === "*") {
      const e = sql.indexOf("*/", i + 2);
      j = e === -1 ? n : e + 2;
      push("comment", i, j);
    } else if (c === "'" || c === '"' || c === "`") {
      j = closing(sql, i, c);
      const closed = j > i + 1 && sql[j - 1] === c && j <= n;
      const inner = sql.slice(i + 1, closed ? j - 1 : j).split(c + c).join(c);
      push(c === "'" ? "string" : "quoted", i, j, inner);
    } else if (/[A-Za-z_\u0080-￿]/.test(c)) {
      while (j < n && /[\w$\u0080-￿]/.test(sql[j])) j++;
      push("word", i, j);
    } else if (/[0-9]/.test(c)) {
      while (j < n && /[\w.]/.test(sql[j])) j++;
      push("other", i, j);
    } else {
      push("punct", i, j);
    }
    i = j;
  }
  return out;
}

function closing(sql: string, open: number, quote: string): number {
  let j = open + 1;
  while (j < sql.length) {
    if (sql[j] === quote) {
      if (sql[j + 1] === quote) {
        j += 2;
        continue;
      }
      return j + 1;
    }
    j++;
  }
  return sql.length;
}
