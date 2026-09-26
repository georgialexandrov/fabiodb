import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Grid } from "./Grid";
import type { ResultColumn } from "./api";

const ROWS = 100_000;
const COLUMNS: ResultColumn[] = Array.from({ length: 12 }, (_, i) => ({
  name: `column_${i}`,
  data_type: i % 3 === 0 ? "integer" : "text",
}));
const row = (i: number) => COLUMNS.map((c, j) => (c.data_type === "integer" ? String(i * 7 + j) : `value ${i}-${j} lorem ipsum`));
const SAMPLE = Array.from({ length: 100 }, (_, i) => row(i));

const SCROLL_PX_PER_FRAME = 240; // a fast trackpad fling, ~14k px/s at 60 fps
const DURATION_MS = 3000;

/**
 * FABIO_BENCH=scroll: the PLAN.md budget "scroll a 100k-row result at 60 fps,
 * no blank rows". Scrolls the real Grid over synthetic rows and reports frame
 * times to bench/startup.py.
 */
export function ScrollBench() {
  useEffect(() => {
    const scroller = document.querySelector<HTMLElement>(".grid");
    if (!scroller) return;
    const frames: number[] = [];
    let last = performance.now();
    const started = last;
    let blank = 0;
    function step(now: number) {
      frames.push(now - last);
      last = now;
      scroller!.scrollTop += SCROLL_PX_PER_FRAME;
      // A row slot with no cells would be a blank row.
      if (scroller!.querySelectorAll(".grid-row.pending").length) blank++;
      if (now - started < DURATION_MS) requestAnimationFrame(step);
      else report();
    }
    function report() {
      const sorted = frames.slice(1).sort((a, b) => a - b);
      const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
      const dropped = sorted.filter((f) => f > 25).length / sorted.length;
      invoke("bench_result", {
        text: `scroll_frames=${sorted.length} p50_ms=${at(0.5).toFixed(1)} p95_ms=${at(0.95).toFixed(1)} max_ms=${sorted[sorted.length - 1].toFixed(1)} dropped_pct=${(dropped * 100).toFixed(1)} blank_frames=${blank}`,
      });
    }
    // Let the first render settle before measuring.
    setTimeout(() => requestAnimationFrame(step), 500);
  }, []);

  return (
    <div className="table-view" style={{ height: "100vh" }}>
      <Grid columns={COLUMNS} rowCount={ROWS} row={row} sample={SAMPLE} />
    </div>
  );
}
