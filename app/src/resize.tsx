// Panel widths the user drags: CSS variables on the root, so dragging restyles
// without re-rendering anything. Kept between launches.

const SIZES = {
  sidebar: { variable: "--sidebar-w", initial: 240, min: 180, max: 480, grows: 1 },
  /** Row details and the value panel, on the right; they share one width. */
  panel: { variable: "--panel-w", initial: 340, min: 240, max: 900, grows: -1 },
} as const;

type Name = keyof typeof SIZES;
const storageKey = (name: Name) => `fabio.width.${name}`;

function set(name: Name, width: number) {
  const s = SIZES[name];
  const max = Math.min(s.max, window.innerWidth * 0.6);
  const w = Math.round(Math.min(max, Math.max(s.min, width)));
  document.documentElement.style.setProperty(s.variable, `${w}px`);
  return w;
}

/** Before the first render, so panels open at their saved width. */
export function applySavedWidths() {
  for (const name of Object.keys(SIZES) as Name[]) {
    const saved = Number(localStorage.getItem(storageKey(name)));
    if (saved) set(name, saved);
  }
}

/** A thin strip on the panel's inner edge: drag to resize, double-click for the default. */
export function ResizeHandle({ name }: { name: Name }) {
  const s = SIZES[name];
  function start(e: React.PointerEvent<HTMLDivElement>) {
    if (e.button !== 0) return;
    e.preventDefault();
    const panel = e.currentTarget.parentElement!;
    const [x0, w0] = [e.clientX, panel.getBoundingClientRect().width];
    let width = w0;
    document.body.classList.add("resizing");
    const move = (m: PointerEvent) => (width = set(name, w0 + (m.clientX - x0) * s.grows));
    const up = () => {
      document.body.classList.remove("resizing");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      localStorage.setItem(storageKey(name), String(width));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
  function reset() {
    set(name, s.initial);
    localStorage.removeItem(storageKey(name));
  }
  return (
    <div
      className={`resize-handle ${s.grows > 0 ? "right" : "left"}`}
      onPointerDown={start}
      onDoubleClick={reset}
      title="Drag to resize; double-click for the default width"
    />
  );
}
