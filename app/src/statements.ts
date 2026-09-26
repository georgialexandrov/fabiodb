/** A statement's trimmed text and where it sits in the editor. */
export type Statement = { from: number; to: number; text: string };

const DOLLAR_TAG = /^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/;

/**
 * Splits SQL on top-level semicolons. Knows about quotes, identifiers, line and
 * block comments, and Postgres dollar quoting — enough to find statement
 * boundaries, not a parser.
 */
export function splitStatements(sql: string): Statement[] {
  const out: Statement[] = [];
  const n = sql.length;
  let start = 0;
  let i = 0;

  const push = (from: number, to: number) => {
    while (from < to && /\s/.test(sql[from])) from++;
    while (to > from && /\s/.test(sql[to - 1])) to--;
    if (to > from) out.push({ from, to, text: sql.slice(from, to) });
  };

  while (i < n) {
    const c = sql[i];
    if (c === "'" || c === '"' || c === "`") {
      i = skipQuoted(sql, i, c);
    } else if (c === "-" && sql[i + 1] === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? n : end + 1;
    } else if (c === "/" && sql[i + 1] === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? n : end + 2;
    } else if (c === "$" && DOLLAR_TAG.test(sql.slice(i, i + 64))) {
      const tag = DOLLAR_TAG.exec(sql.slice(i, i + 64))![0];
      const end = sql.indexOf(tag, i + tag.length);
      i = end === -1 ? n : end + tag.length;
    } else if (c === ";") {
      push(start, i);
      start = ++i;
    } else {
      i++;
    }
  }
  push(start, n);
  return out;
}

function skipQuoted(sql: string, open: number, quote: string): number {
  let j = open + 1;
  while (j < sql.length) {
    if (sql[j] === quote) {
      if (sql[j + 1] === quote) {
        j += 2; // doubled quote is an escaped quote
        continue;
      }
      return j + 1;
    }
    j++;
  }
  return sql.length;
}

/** The statement under `pos`, or the nearest one before it. */
export function statementAt(statements: Statement[], pos: number): Statement | undefined {
  let before: Statement | undefined;
  for (const s of statements) {
    if (pos >= s.from && pos <= s.to + 1) return s; // +1: just after the `;`
    if (s.to < pos) before = s;
  }
  return before ?? statements[0];
}
