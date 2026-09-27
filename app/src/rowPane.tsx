import { useSyncExternalStore } from "react";

// Whether the row details pane is open. One setting for every grid, kept between launches.
const KEY = "fabio.rowPane";
const listeners = new Set<() => void>();
let open = (() => {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
})();

export function toggleRowPane() {
  open = !open;
  try {
    localStorage.setItem(KEY, open ? "1" : "0");
  } catch {
    // Remembered until quit, then.
  }
  listeners.forEach((l) => l());
}

export function useRowPane(): boolean {
  return useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => open,
  );
}

/** The toolbar button: a window with its right side panel. */
export function RowPaneButton() {
  const on = useRowPane();
  return (
    <button className={`ghost icon ${on ? "on" : ""}`} onClick={toggleRowPane} title="Row details (⌘I)" aria-pressed={on}>
      <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
        <rect x="1.5" y="2.5" width="13" height="11" rx="2" fill="none" stroke="currentColor" strokeWidth="1.3" />
        <path d="M10 2.5v11" stroke="currentColor" strokeWidth="1.3" />
        {on && <rect x="10" y="3" width="4" height="10" rx="1" fill="currentColor" opacity="0.35" />}
      </svg>
    </button>
  );
}
