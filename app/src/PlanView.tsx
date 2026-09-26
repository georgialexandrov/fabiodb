import { useState } from "react";
import type { Plan, PlanNode } from "./api";

type Props = {
  plan: Plan;
  /** The previous plan of the same statement, for a before/after line. */
  previous: Plan | null;
};

export function PlanView({ plan, previous }: Props) {
  const [focus, setFocus] = useState<string | null>(plan.findings[0] ? key(plan.findings[0].path) : null);
  const flagged = new Map(plan.findings.map((f) => [key(f.path), f.severity]));
  const scale = plan.execution_ms ?? plan.root.total_ms ?? 0;

  return (
    <div className="plan">
      <div className="plan-head">
        {plan.analyzed ? (
          <span>
            Ran in <b>{ms(plan.execution_ms)}</b>
            {plan.planning_ms != null && <span className="muted"> · planned in {ms(plan.planning_ms)}</span>}
          </span>
        ) : (
          <span className="muted">Estimate only — the statement didn't run. ⌘E runs it (rolled back).</span>
        )}
        {previous?.execution_ms != null && plan.execution_ms != null && (
          <span className="compare">
            before {ms(previous.execution_ms)} → now {ms(plan.execution_ms)}
            <b> {ratio(previous.execution_ms, plan.execution_ms)}</b>
          </span>
        )}
        <span className="grow" />
        <button className="ghost" onClick={() => navigator.clipboard.writeText(plan.raw)} title="Copy the raw plan (for explain.dalibo.com and friends)">
          Copy raw
        </button>
      </div>

      {plan.findings.length > 0 && (
        <ul className="findings">
          {plan.findings.map((f, i) => (
            <li key={i} className={f.severity} onClick={() => setFocus(key(f.path))}>
              {f.message}
            </li>
          ))}
        </ul>
      )}

      <div className="plan-tree">
        <Node node={plan.root} path={[]} depth={0} scale={scale} flagged={flagged} focus={focus} />
      </div>
    </div>
  );
}

type NodeProps = {
  node: PlanNode;
  path: number[];
  depth: number;
  scale: number;
  flagged: Map<string, "hot" | "warn">;
  focus: string | null;
};

function Node({ node, path, depth, scale, flagged, focus }: NodeProps) {
  const k = key(path);
  const severity = flagged.get(k);
  const share = node.self_ms != null && scale > 0 ? Math.min(1, node.self_ms / scale) : null;
  const perLoop = node.actual_rows != null ? node.actual_rows / Math.max(1, node.loops ?? 1) : null;
  const off = node.estimated_rows != null && perLoop != null ? misestimate(node.estimated_rows, perLoop) : null;
  // SQLite has no numbers; a full-table SCAN is the thing to notice.
  const fullScan = /^SCAN /.test(node.operation) && !/INDEX/.test(node.operation);
  // Postgres reports buffers including children; show what this node touched itself.
  const own = (k: "shared_hit" | "shared_read") =>
    Math.max(0, (node[k] ?? 0) - node.children.reduce((sum, c) => sum + (c[k] ?? 0), 0));
  const loops = node.loops ?? 1;

  return (
    <>
      <div
        className={`plan-node ${severity ?? ""} ${focus === k ? "focus" : ""}`}
        style={{ paddingLeft: 12 + depth * 18 }}
        ref={(el) => {
          if (el && focus === k) el.scrollIntoView({ block: "nearest" });
        }}
      >
        <div className="plan-line">
          <span className={`op ${fullScan ? "full-scan" : ""}`}>{node.operation}</span>
          {node.target && <span className="muted"> {node.target}</span>}
          <span className="grow" />
          {perLoop != null && (
            <span className="rows" title={`estimated ${fmt(node.estimated_rows)} per loop`}>
              {fmt(node.actual_rows)} rows
              {(node.loops ?? 1) > 1 && <span className="muted"> · {fmt(node.loops)} loops</span>}
              {off && <span className="off"> · est. {fmt(node.estimated_rows)} ({off})</span>}
            </span>
          )}
          {perLoop == null && node.estimated_rows != null && <span className="rows muted">~{fmt(node.estimated_rows)} rows</span>}
          {node.self_ms != null && (
            <span className="time">
              <span className="bar">
                <span style={{ width: `${(share ?? 0) * 100}%` }} />
              </span>
              {ms(node.self_ms)}
            </span>
          )}
        </div>
        {(node.details.length > 0 || own("shared_hit") + own("shared_read") > 0) && (
          <div className="plan-details">
            {node.details.map((d) => (
              <div key={d.label}>
                <span className="muted">{d.label}:</span> {detailValue(d.label, d.value, loops)}
              </div>
            ))}
            {own("shared_hit") + own("shared_read") > 0 && (
              <div>
                <span className="muted">Buffers:</span> {fmt(own("shared_hit"))} cached, {fmt(own("shared_read"))} read
              </div>
            )}
          </div>
        )}
      </div>
      {node.children.map((c, i) => (
        <Node key={i} node={c} path={[...path, i]} depth={depth + 1} scale={scale} flagged={flagged} focus={focus} />
      ))}
    </>
  );
}

const key = (path: number[]) => path.join(".");

/** Postgres gives row counts per loop; show the total people expect. */
function detailValue(label: string, value: string, loops: number) {
  if (!/^\d+$/.test(value)) return value;
  const n = Number(value) * (label.startsWith("Rows removed") ? loops : 1);
  return n.toLocaleString();
}

function fmt(n: number | null | undefined) {
  return n == null ? "–" : Math.round(n).toLocaleString();
}

function ms(n: number | null | undefined) {
  if (n == null) return "–";
  if (n >= 1000) return `${(n / 1000).toFixed(2)} s`;
  if (n >= 10) return `${n.toFixed(0)} ms`;
  return `${n.toFixed(2)} ms`;
}

function misestimate(est: number, actual: number) {
  const hi = Math.max(est, actual);
  const lo = Math.max(1, Math.min(est, actual));
  return hi >= 100 && hi / lo >= 10 ? `${Math.round(hi / lo)}× ${actual > est ? "more" : "fewer"}` : null;
}

function ratio(before: number, now: number) {
  if (now <= 0 || before <= 0) return "";
  const r = before / now;
  if (r >= 1.1) return `${r.toFixed(r >= 10 ? 0 : 1)}× faster`;
  if (r <= 0.9) return `${(1 / r).toFixed(1)}× slower`;
  return "about the same";
}
