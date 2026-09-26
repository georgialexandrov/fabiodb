import { useEffect, useMemo, useRef, useState } from "react";
import { fuzzyFilter } from "./fuzzy";
import type { SavedConnection } from "./api";

const RECENT_KEY = "fabio.recentConnections";

/** Ids of recently used connections, newest first. */
export function recentConnections(): string[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
  } catch {
    return [];
  }
}

export function markUsed(id: string) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify([id, ...recentConnections().filter((x) => x !== id)].slice(0, 20)));
  } catch {
    // Recents just won't be remembered.
  }
}

export const describe = (c: SavedConnection) =>
  c.target.engine === "postgres" ? `${c.target.user}@${c.target.host}:${c.target.port}/${c.target.database}` : c.target.path;

type Props = {
  connections: SavedConnection[];
  openIds: Set<string>;
  activeId: string | null;
  /** "page" fills the main area when nothing is open; "popover" drops from the sidebar. */
  mode: "page" | "popover";
  onPick: (c: SavedConnection) => void;
  onEdit: (c: SavedConnection) => void;
  onNew: () => void;
  onOpenSqlite: () => void;
  /** Offered only once Docker scanning exists. */
  onScanFolder?: () => void;
  onClose?: () => void;
};

type Section = { title: string; items: SavedConnection[] };

/** Every saved connection, searchable: open ones, recent ones, then by group. */
export function ConnectionSwitcher(props: Props) {
  const { connections, openIds, activeId, mode, onPick, onEdit, onNew, onOpenSqlite, onScanFolder, onClose } = props;
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const root = useRef<HTMLDivElement>(null);

  const sections: Section[] = useMemo(() => {
    if (query.trim()) {
      const text = (c: SavedConnection) => `${c.name} ${c.group ?? ""} ${describe(c)}`;
      return [{ title: "", items: fuzzyFilter(connections, query, text) }];
    }
    const byId = new Map(connections.map((c) => [c.id, c]));
    const open = connections.filter((c) => openIds.has(c.id));
    const recent = recentConnections()
      .map((id) => byId.get(id))
      .filter((c): c is SavedConnection => !!c && !openIds.has(c.id))
      .slice(0, 5);
    const shown = new Set([...open, ...recent].map((c) => c.id));
    const groups = new Map<string, SavedConnection[]>();
    for (const c of connections) {
      if (shown.has(c.id)) continue;
      const g = c.group?.trim() || "Connections";
      groups.set(g, [...(groups.get(g) ?? []), c]);
    }
    const named = [...groups.keys()].filter((g) => g !== "Connections").sort((a, b) => a.localeCompare(b));
    return [
      { title: "Open", items: open },
      { title: "Recent", items: recent },
      ...named.map((g) => ({ title: g, items: groups.get(g)! })),
      { title: named.length ? "Other" : "Connections", items: groups.get("Connections") ?? [] },
    ].filter((s) => s.items.length > 0);
  }, [connections, openIds, query]);

  const flat = sections.flatMap((s) => s.items);
  useEffect(() => setIndex(0), [query]);

  useEffect(() => {
    if (mode !== "popover") return;
    const close = (e: MouseEvent) => !root.current?.contains(e.target as Node) && onClose?.();
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [mode, onClose]);

  useEffect(() => {
    root.current?.querySelector(".switcher-item.active")?.scrollIntoView({ block: "nearest" });
  }, [index]);

  let n = 0;
  return (
    <div className={`switcher ${mode}`} ref={root}>
      <input
        autoFocus
        spellCheck={false}
        placeholder={`Search ${connections.length} connection${connections.length === 1 ? "" : "s"}`}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") setIndex((i) => Math.min(i + 1, flat.length - 1));
          else if (e.key === "ArrowUp") setIndex((i) => Math.max(i - 1, 0));
          else if (e.key === "Enter" && flat[index]) onPick(flat[index]);
          else if (e.key === "Escape") onClose?.();
          else return;
          e.preventDefault();
          e.stopPropagation();
        }}
      />
      <div className="switcher-list">
        {flat.length === 0 && <p className="hint">{query ? `Nothing matches “${query}”.` : "No connections yet."}</p>}
        {sections.map((section) => (
          <div key={section.title}>
            {section.title && <div className="switcher-section">{section.title}</div>}
            {section.items.map((c) => {
              const i = n++;
              return (
                <div
                  key={c.id}
                  className={`switcher-item ${i === index ? "active" : ""} ${c.id === activeId ? "current" : ""}`}
                  onMouseMove={() => setIndex(i)}
                  onClick={() => onPick(c)}
                >
                  <span className={`engine ${c.target.engine}`}>{c.target.engine === "postgres" ? "PG" : "SQ"}</span>
                  <span className="switcher-text">
                    <span className="ellipsis">{c.name}</span>
                    <span className="muted ellipsis switcher-sub">{describe(c)}</span>
                  </span>
                  {openIds.has(c.id) && <span className="dot" title="Connected" />}
                  <button
                    className="ghost edit"
                    title="Edit connection"
                    onClick={(e) => {
                      e.stopPropagation();
                      onEdit(c);
                    }}
                  >
                    ⋯
                  </button>
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <div className="switcher-actions">
        <button className="ghost" onClick={onNew}>
          New connection <kbd>⌘N</kbd>
        </button>
        <button className="ghost" onClick={onOpenSqlite}>
          SQLite file <kbd>⌘O</kbd>
        </button>
        {onScanFolder && (
          <button className="ghost" onClick={onScanFolder}>
            Docker folder…
          </button>
        )}
      </div>
    </div>
  );
}
