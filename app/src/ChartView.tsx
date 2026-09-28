import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Rows } from "./api";
import { allowed, columnKinds, niceTicks, pick, reshape, toTime, type Form, type Kind, type Spec } from "./chartSpec";

/** More bars than this stop being readable; the rest stay in Results. */
const BAR_CAP = 40;
/** Colour-blind safe as a set in both themes; past three, lines fold into Results. */
const SERIES = ["var(--series-1)", "var(--series-2)", "var(--series-3)"];

type Props = { columns: string[]; rows: Rows };
/** Where the pointer is on screen, and what to say there. */
type Tip = { x: number; y: number; body: ReactNode } | null;

export default function ChartView({ columns, rows }: Props) {
  const kinds = useMemo(() => columnKinds(columns, rows), [columns, rows]);
  // A new result re-picks the chart unless it has the same columns: re-running keeps your choice.
  const shape = columns.join("\u0000");
  const [state, setState] = useState<{ shape: string; spec: Spec }>(() => ({ shape, spec: pick(columns, kinds, rows) }));
  const spec = state.shape === shape ? state.spec : pick(columns, kinds, rows);
  const set = (next: Spec) => setState({ shape, spec: next });

  const host = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = host.current!;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const [tip, setTip] = useState<Tip>(null);

  const choosable = spec.form === "line" || spec.form === "bar" || spec.form === "scatter";
  const column = (label: string, key: "x" | "y" | "series", fits: (k: Kind) => boolean, none = false) => (
    <label>
      {label}{" "}
      <select value={spec[key] ?? ""} onChange={(e) => set({ ...spec, [key]: e.target.value === "" ? null : +e.target.value, why: "" })}>
        {none && <option value="">—</option>}
        {columns.map((c, i) => fits(kinds[i]) && <option key={i} value={i}>{c}</option>)}
      </select>
    </label>
  );

  const props = { columns, rows, kinds, spec, width, setTip };
  return (
    <div className="chart">
      <div className="chart-head">
        <div className="segmented">
          {(["line", "bar", "scatter"] as Form[]).map((f) => (
            <button
              key={f}
              className={spec.form === f ? "on" : ""}
              disabled={!allowed(f, kinds)}
              title={allowed(f, kinds) ? undefined : f === "scatter" ? "Needs two number columns" : "Needs a number and another column"}
              onClick={() => set(reshape(spec, f, kinds))}
            >
              {f[0].toUpperCase() + f.slice(1)}
            </button>
          ))}
        </div>
        {choosable && column("x", "x", (k) => spec.form !== "scatter" || k === "number")}
        {choosable && column("y", "y", (k) => k === "number")}
        {spec.form === "line" && column("per", "series", (k) => k === "text" || k === "key", true)}
        <span className="muted ellipsis">{spec.why}</span>
      </div>
      <div className="chart-body" ref={host} onMouseLeave={() => setTip(null)}>
        {width > 0 && spec.form === "line" && <LineChart {...props} />}
        {width > 0 && spec.form === "bar" && <BarChart {...props} />}
        {width > 0 && spec.form === "scatter" && <ScatterChart {...props} />}
        {spec.form === "value" && (
          <div className="chart-value">
            <b>{fmt(rows[0][spec.y!])}</b>
            <span className="muted">{columns[spec.y!]}</span>
          </div>
        )}
        {spec.form === "none" && <p className="hint">{spec.why}</p>}
        {tip && (
          <div className="chart-tip" style={{ left: Math.min(tip.x + 14, window.innerWidth - 280), top: tip.y + 14 }}>
            {tip.body}
          </div>
        )}
      </div>
    </div>
  );
}

type ChartProps = { columns: string[]; rows: Rows; kinds: Kind[]; spec: Spec; width: number; setTip: (t: Tip) => void };

const fmt = (v: string | number | null) => (v === null ? "NULL" : Number(v).toLocaleString(undefined, { maximumFractionDigits: 2 }));

/** How an x value becomes a position, and how it reads back. */
function xAxis(rows: Rows, col: number, kind: Kind) {
  if (kind === "time") {
    const dateOnly = rows.every((r) => r[col] === null || r[col]!.length === 10);
    const all = rows.flatMap((r) => (r[col] === null ? [] : [toTime(r[col]!)]));
    const span = Math.max(...all) - Math.min(...all);
    const opts: Intl.DateTimeFormatOptions = dateOnly
      ? span > 400 * 864e5
        ? { month: "short", year: "numeric", timeZone: "UTC" }
        : { month: "short", day: "numeric", timeZone: "UTC" }
      : span > 2 * 864e5
        ? { month: "short", day: "numeric" }
        : { hour: "2-digit", minute: "2-digit" };
    return { value: (v: string) => toTime(v), label: (n: number) => new Date(n).toLocaleString(undefined, opts) };
  }
  if (kind === "number") return { value: (v: string) => +v, label: (n: number) => fmt(n) };
  // Text along a line: evenly spaced, in the order the rows came.
  const order = [...new Set(rows.map((r) => r[col] ?? "NULL"))];
  const at = new Map(order.map((v, i) => [v, i]));
  return { value: (v: string) => at.get(v)!, label: (n: number) => order[n] ?? "" };
}

function YGrid({ ticks, Y, left, right }: { ticks: number[]; Y: (v: number) => number; left: number; right: number }) {
  return (
    <>
      {ticks.map((t) => (
        <g key={t}>
          <line x1={left} x2={right} y1={Y(t)} y2={Y(t)} className={t === 0 ? "axis" : "grid"} />
          <text x={left - 8} y={Y(t) + 4} textAnchor="end">
            {fmt(t)}
          </text>
        </g>
      ))}
    </>
  );
}

function LineChart({ columns, rows, kinds, spec, width, setTip }: ChartProps) {
  const x = spec.x!, y = spec.y!, by = spec.series;
  const [hover, setHover] = useState<number | null>(null);
  const axis = useMemo(() => xAxis(rows, x, kinds[x]), [rows, x, kinds]);
  const { names, groups, folded } = useMemo(() => {
    const groups = new Map<string, { x: number; y: number }[]>();
    for (const r of rows) {
      if (r[x] === null || r[y] === null) continue;
      const key = by === null ? columns[y] : (r[by] ?? "NULL");
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push({ x: axis.value(r[x]!), y: +r[y]! });
    }
    for (const g of groups.values()) g.sort((a, b) => a.x - b.x);
    let names = [...groups.keys()];
    const folded = Math.max(0, names.length - SERIES.length);
    if (folded) {
      const total = (n: string) => groups.get(n)!.reduce((a, p) => a + Math.abs(p.y), 0);
      const keep = new Set([...names].sort((a, b) => total(b) - total(a)).slice(0, SERIES.length));
      names = names.filter((n) => keep.has(n));
    }
    return { names, groups, folded };
  }, [rows, x, y, by, axis, columns]);

  const multi = by !== null;
  const points = names.flatMap((n) => groups.get(n)!);
  if (!points.length) return <p className="hint">Every row has a NULL in {columns[x]} or {columns[y]}.</p>;
  const H = 320, m = { l: 60, r: multi ? 80 : 20, t: 12, b: 28 };
  const xs = [...new Set(points.map((p) => p.x))].sort((a, b) => a - b);
  const x0 = xs[0], x1 = xs[xs.length - 1];
  const ticks = niceTicks(Math.max(...points.map((p) => p.y)), Math.min(...points.map((p) => p.y)));
  const X = (v: number) => m.l + ((v - x0) / (x1 - x0 || 1)) * (width - m.l - m.r);
  const Y = (v: number) => H - m.b - ((v - ticks[0]) / (ticks[ticks.length - 1] - ticks[0])) * (H - m.t - m.b);
  const every = Math.ceil(xs.length / Math.max(2, Math.floor((width - m.l - m.r) / 90)));
  const colour = (i: number) => (multi ? SERIES[i] : "var(--mark)");

  // Labels at the line ends, nudged apart when two lines finish close together.
  const ends = names
    .map((n) => {
      const g = groups.get(n)!, last = g[g.length - 1];
      return { n, x: X(last.x) + 8, y: Y(last.y) + 4 };
    })
    .sort((a, b) => a.y - b.y);
  ends.forEach((e, i) => i && e.y - ends[i - 1].y < 13 && (e.y = ends[i - 1].y + 13));

  const move = (e: React.MouseEvent<SVGRectElement>) => {
    const px = e.nativeEvent.offsetX;
    const v = xs.reduce((a, b) => (Math.abs(X(b) - px) < Math.abs(X(a) - px) ? b : a));
    setHover(v);
    setTip({
      x: e.clientX,
      y: e.clientY,
      body: (
        <>
          <div className="muted">{axis.label(v)}</div>
          {names.map((n, i) => {
            const p = groups.get(n)!.find((p) => p.x === v);
            return p && (
              <div key={n}>
                <i style={{ background: multi ? colour(i) : "var(--accent)" }} />
                {multi && `${n} `}
                <b>{fmt(p.y)}</b>
              </div>
            );
          })}
        </>
      ),
    });
  };

  return (
    <>
      <div className="chart-title">
        {columns[y]} by {columns[x]}
        {multi && `, per ${columns[by!]}`}
      </div>
      {multi && (
        <div className="chart-legend">
          {names.map((n, i) => (
            <span key={n} style={{ "--c": colour(i) } as React.CSSProperties}>
              {n}
            </span>
          ))}
          {folded > 0 && <span className="muted">{folded} more in Results</span>}
        </div>
      )}
      <svg width={width} height={H}>
        <YGrid ticks={ticks} Y={Y} left={m.l} right={width - m.r} />
        {xs.map((v, i) => i % every === 0 && (
          <text key={v} x={X(v)} y={H - 8} textAnchor="middle">
            {axis.label(v)}
          </text>
        ))}
        {names.map((n, i) => (
          <path key={n} className="series" stroke={colour(i)} d={"M" + groups.get(n)!.map((p) => `${X(p.x)},${Y(p.y)}`).join("L")} />
        ))}
        {multi && ends.map((e) => (
          <text key={e.n} x={e.x} y={e.y} className="label">
            {e.n}
          </text>
        ))}
        {hover !== null && (
          <>
            <line className="crosshair" x1={X(hover)} x2={X(hover)} y1={m.t} y2={H - m.b} />
            {names.map((n, i) => {
              const p = groups.get(n)!.find((p) => p.x === hover);
              return p && <circle key={n} className="dot" cx={X(p.x)} cy={Y(p.y)} r={4.5} fill={multi ? colour(i) : "var(--accent)"} />;
            })}
          </>
        )}
        <rect x={m.l} y={m.t} width={Math.max(0, width - m.l - m.r)} height={H - m.t - m.b} fill="transparent" onMouseMove={move} onMouseLeave={() => (setHover(null), setTip(null))} />
      </svg>
    </>
  );
}

function BarChart({ columns, rows, spec, width, setTip }: ChartProps) {
  const x = spec.x!, y = spec.y!;
  const shown = rows.slice(0, BAR_CAP).filter((r) => r[y] !== null);
  const [hover, setHover] = useState<number | null>(null);
  const band = 22, m = { l: 160, r: 64, t: 22, b: 6 };
  const H = m.t + shown.length * band + m.b;
  const values = shown.map((r) => +r[y]!);
  const ticks = niceTicks(Math.max(...values), Math.min(...values));
  const X = (v: number) => m.l + ((v - ticks[0]) / (ticks[ticks.length - 1] - ticks[0])) * (width - m.l - m.r);
  const zero = X(0);

  return (
    <>
      <div className="chart-title">
        {columns[y]} by {columns[x]}
      </div>
      {rows.length > BAR_CAP && <p className="hint">The first {BAR_CAP} of {rows.length.toLocaleString()} rows; the rest are in Results.</p>}
      <svg width={width} height={H}>
        {ticks.map((t) => (
          <g key={t}>
            <line x1={X(t)} x2={X(t)} y1={m.t - 4} y2={H - m.b} className={t === 0 ? "axis" : "grid"} />
            <text x={X(t)} y={12} textAnchor="middle">
              {fmt(t)}
            </text>
          </g>
        ))}
        {shown.map((r, i) => {
          const v = +r[y]!, top = m.t + i * band + 2, h = band - 4;
          const label = r[x] ?? "NULL";
          const end = X(v), w = Math.abs(end - zero), rad = Math.min(4, w);
          // Square at the baseline, rounded at the value end.
          const d =
            v >= 0
              ? `M${zero},${top}h${w - rad}a${rad},${rad} 0 0 1 ${rad},${rad}v${h - 2 * rad}a${rad},${rad} 0 0 1 -${rad},${rad}h-${w - rad}z`
              : `M${zero},${top}h-${w - rad}a${rad},${rad} 0 0 0 -${rad},${rad}v${h - 2 * rad}a${rad},${rad} 0 0 0 ${rad},${rad}h${w - rad}z`;
          return (
            <g
              key={i}
              onMouseMove={(e) => {
                setHover(i);
                setTip({ x: e.clientX, y: e.clientY, body: <><div className="muted">{label}</div><b>{fmt(v)}</b> <span className="muted">{columns[y]}</span></> });
              }}
              onMouseLeave={() => (setHover(null), setTip(null))}
            >
              <rect x={0} y={top - 2} width={width} height={band} fill="transparent" />
              <text x={m.l - 8} y={top + h / 2 + 4} textAnchor="end" className="label">
                {label.length > 24 ? label.slice(0, 23) + "…" : label}
              </text>
              <path d={d} fill={hover === i ? "var(--accent)" : "var(--mark)"} />
              {(i === 0 || hover === i) && (
                <text x={v >= 0 ? end + 6 : end - 6} y={top + h / 2 + 4} textAnchor={v >= 0 ? "start" : "end"}>
                  {fmt(v)}
                </text>
              )}
            </g>
          );
        })}
      </svg>
    </>
  );
}

function ScatterChart({ columns, rows, spec, width, setTip }: ChartProps) {
  const x = spec.x!, y = spec.y!, label = spec.label;
  const H = 360, m = { l: 60, r: 20, t: 12, b: 38 };
  const shown = useMemo(() => rows.filter((r) => r[x] !== null && r[y] !== null), [rows, x, y]);
  const xv = shown.map((r) => +r[x]!), yv = shown.map((r) => +r[y]!);
  const xt = niceTicks(Math.max(...xv), Math.min(...xv)), yt = niceTicks(Math.max(...yv), Math.min(...yv));
  const X = (v: number) => m.l + ((v - xt[0]) / (xt[xt.length - 1] - xt[0])) * (width - m.l - m.r);
  const Y = (v: number) => H - m.b - ((v - yt[0]) / (yt[yt.length - 1] - yt[0])) * (H - m.t - m.b);
  const [hot, setHot] = useState<number | null>(null);

  const move = (e: React.MouseEvent<SVGRectElement>) => {
    const { offsetX: px, offsetY: py } = e.nativeEvent;
    let best = -1, bd = 14 ** 2;
    for (let i = 0; i < shown.length; i++) {
      const d = (X(xv[i]) - px) ** 2 + (Y(yv[i]) - py) ** 2;
      if (d < bd) (bd = d), (best = i);
    }
    if (best < 0) return setHot(null), setTip(null);
    setHot(best);
    const r = shown[best];
    setTip({
      x: e.clientX,
      y: e.clientY,
      body: (
        <>
          {label !== null && <div>{r[label] ?? "NULL"}</div>}
          <div><span className="muted">{columns[x]}</span> <b>{fmt(r[x])}</b></div>
          <div><span className="muted">{columns[y]}</span> <b>{fmt(r[y])}</b></div>
        </>
      ),
    });
  };

  return (
    <>
      <div className="chart-title">
        {columns[y]} against {columns[x]}
      </div>
      <svg width={width} height={H}>
        <YGrid ticks={yt} Y={Y} left={m.l} right={width - m.r} />
        {xt.map((t) => (
          <text key={t} x={X(t)} y={H - 20} textAnchor="middle">
            {fmt(t)}
          </text>
        ))}
        <text x={width - m.r} y={H - 3} textAnchor="end">
          {columns[x]} →
        </text>
        <text x={m.l + 8} y={m.t + 12}>
          ↑ {columns[y]}
        </text>
        {shown.map((_, i) => (
          <circle key={i} className="point" cx={X(xv[i])} cy={Y(yv[i])} r={4} />
        ))}
        {hot !== null && <circle className="dot" cx={X(xv[hot])} cy={Y(yv[hot])} r={5.5} fill="var(--accent)" />}
        <rect x={m.l} y={m.t} width={Math.max(0, width - m.l - m.r)} height={H - m.t - m.b} fill="transparent" onMouseMove={move} onMouseLeave={() => (setHot(null), setTip(null))} />
      </svg>
    </>
  );
}
