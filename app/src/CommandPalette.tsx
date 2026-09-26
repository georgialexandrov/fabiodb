import { useEffect, useMemo, useRef, useState } from "react";
import { fuzzyFilter } from "./fuzzy";

export type Command = {
  id: string;
  label: string;
  /** Muted text after the label: a table's kind, a connection's engine. */
  hint?: string;
  shortcut?: string;
  run: () => void;
};

const SHOWN = 60;

/** ⌘K: type to find a table, a connection or an action; ↵ runs it. */
export function CommandPalette({ commands, onClose }: { commands: Command[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const matches = useMemo(
    () => fuzzyFilter(commands, query, (c) => `${c.label} ${c.hint ?? ""}`).slice(0, SHOWN),
    [commands, query],
  );

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.children[index]?.scrollIntoView({ block: "nearest" });
  }, [index]);

  function run(command: Command | undefined) {
    if (!command) return;
    onClose();
    command.run();
  }

  return (
    <div className="modal-backdrop palette-backdrop" onMouseDown={onClose}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <input
          autoFocus
          spellCheck={false}
          placeholder="Table, connection or action"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") setIndex((i) => Math.min(i + 1, matches.length - 1));
            else if (e.key === "ArrowUp") setIndex((i) => Math.max(i - 1, 0));
            else if (e.key === "Enter") run(matches[index]);
            else if (e.key === "Escape") onClose();
            else return;
            e.preventDefault();
            e.stopPropagation();
          }}
        />
        <div className="palette-list" ref={list}>
          {matches.length === 0 && <p className="hint">Nothing matches “{query}”.</p>}
          {matches.map((c, i) => (
            <div
              key={c.id}
              className={`palette-item ${i === index ? "active" : ""}`}
              onMouseMove={() => setIndex(i)}
              onClick={() => run(c)}
            >
              <span className="ellipsis">{c.label}</span>
              {c.hint && <span className="muted palette-hint">{c.hint}</span>}
              <span className="grow" />
              {c.shortcut && <kbd>{c.shortcut}</kbd>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
