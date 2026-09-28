// Loaded lazily (see QueryTab) so CodeMirror stays out of the startup bundle.
import { useEffect, useRef } from "react";
import { Compartment, EditorState, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, keymap, lineNumbers, placeholder } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { PostgreSQL, SQLite, sql } from "@codemirror/lang-sql";
import {
  autocompletion,
  closeBrackets,
  closeCompletion,
  type CompletionContext,
  type CompletionResult,
  completionKeymap,
} from "@codemirror/autocomplete";
import { bracketMatching, HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import type { CompletionTable } from "./api";
import { complete } from "./sqlComplete";

export type EditorSnapshot = { text: string; from: number; to: number; head: number };

export type SqlEditorProps = {
  value: string;
  onChange: (value: string) => void;
  engine: "postgres" | "sqlite";
  schema: CompletionTable[];
  /** The schema picked in the sidebar; its tables are suggested first. */
  activeSchema?: string | null;
  onRun: (mode: "statement" | "all", at: EditorSnapshot) => void;
  onFormat: (at: EditorSnapshot) => void;
  onExplain: (analyze: boolean, at: EditorSnapshot) => void;
  onCancel: () => void;
  /** Underlines this range until the text changes. */
  errorRange: { from: number; to: number } | null;
  /** Called whenever the cursor or selection moves. */
  onSelection?: (at: EditorSnapshot) => void;
};

const setErrorRange = StateEffect.define<{ from: number; to: number } | null>();
const errorMark = Decoration.mark({ class: "cm-sql-error" });
const errorField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    if (tr.docChanged) marks = Decoration.none;
    for (const e of tr.effects) {
      if (e.is(setErrorRange)) {
        const len = tr.state.doc.length;
        const r = e.value;
        marks = r && r.from < len ? Decoration.set([errorMark.range(r.from, Math.min(r.to, len))]) : Decoration.none;
      }
    }
    return marks;
  },
  provide: (f) => EditorView.decorations.from(f),
});

const highlight = HighlightStyle.define([
  { tag: tags.keyword, fontWeight: "600" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--sql-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--sql-number)" },
  { tag: tags.comment, color: "var(--muted)", fontStyle: "italic" },
  { tag: [tags.operator, tags.punctuation], color: "var(--muted)" },
  { tag: tags.typeName, color: "var(--sql-type)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "13px", backgroundColor: "transparent" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.6" },
  ".cm-content": { caretColor: "var(--accent)", padding: "10px 0" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "var(--muted)", opacity: "0.6" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--accent-soft) !important" },
  ".cm-cursor": { borderLeftColor: "var(--accent)" },
  ".cm-placeholder": { color: "var(--muted)" },
  ".cm-tooltip": { border: "1px solid var(--line)", backgroundColor: "var(--surface)", borderRadius: "6px" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--accent-soft)", color: "var(--text)" },
  ".cm-sql-error": { textDecoration: "underline wavy var(--accent)", textUnderlineOffset: "3px" },
});

/** Keywords from lang-sql; tables and columns from what the statement names (sqlComplete). */
function language(engine: SqlEditorProps["engine"], schema: CompletionTable[], activeSchema: string | null) {
  const support = sql({ dialect: engine === "postgres" ? PostgreSQL : SQLite });
  const source = (ctx: CompletionContext): CompletionResult | null => {
    const found = complete(ctx.state.doc.toString(), ctx.pos, schema, { engine, activeSchema });
    if (!found) return null;
    // Nothing typed yet: only when asked (⌃Space) or right after a dot.
    if (found.from === ctx.pos && !ctx.explicit && ctx.state.sliceDoc(ctx.pos - 1, ctx.pos) !== ".") return null;
    return { ...found, validFor: /^"?[\w$]*$/ };
  };
  return [support, support.language.data.of({ autocomplete: source })];
}

function snapshot(view: EditorView): EditorSnapshot {
  const { from, to, head } = view.state.selection.main;
  return { text: view.state.doc.toString(), from, to, head };
}

export default function SqlEditor(props: SqlEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(props);
  latest.current = props;
  const lang = useRef(new Compartment());

  useEffect(() => {
    const v = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: props.value,
        extensions: [
          lineNumbers(),
          EditorView.lineWrapping,
          history(),
          closeBrackets(),
          bracketMatching(),
          autocompletion({ activateOnTyping: true }),
          lang.current.of(language(props.engine, props.schema, props.activeSchema ?? null)),
          syntaxHighlighting(highlight),
          errorField,
          placeholder("select …    ⌘↵ runs the statement under the cursor, ⇧⌘↵ runs all"),
          keymap.of([
            ...completionKeymap,
            { key: "Mod-Enter", run: (v) => (closeCompletion(v), latest.current.onRun("statement", snapshot(v)), true) },
            { key: "Shift-Mod-Enter", run: (v) => (closeCompletion(v), latest.current.onRun("all", snapshot(v)), true) },
            { key: "Mod-e", run: (v) => (closeCompletion(v), latest.current.onExplain(true, snapshot(v)), true) },
            { key: "Shift-Mod-e", run: (v) => (closeCompletion(v), latest.current.onExplain(false, snapshot(v)), true) },
            { key: "Alt-Shift-f", run: (v) => (latest.current.onFormat(snapshot(v)), true) },
            { key: "Escape", run: () => (latest.current.onCancel(), false) },
            ...defaultKeymap,
            ...historyKeymap,
            indentWithTab,
          ]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) latest.current.onChange(u.state.doc.toString());
            if (u.docChanged || u.selectionSet) latest.current.onSelection?.(snapshot(u.view));
          }),
          theme,
        ],
      }),
    });
    view.current = v;
    v.focus();
    return () => v.destroy();
  }, []);

  // Controlled value: replace the document when it changes from outside (history, format).
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== props.value) {
      v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: props.value } });
    }
  }, [props.value]);

  useEffect(() => {
    view.current?.dispatch({ effects: lang.current.reconfigure(language(props.engine, props.schema, props.activeSchema ?? null)) });
  }, [props.engine, props.schema, props.activeSchema]);

  useEffect(() => {
    view.current?.dispatch({ effects: setErrorRange.of(props.errorRange) });
  }, [props.errorRange]);

  return <div className="sql-editor" ref={host} />;
}
