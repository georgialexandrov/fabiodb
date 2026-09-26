# Benchmarks

Run `python3 bench/startup.py` after `cd app && pnpm tauri build --bundles app`.

## 2026-09-26 — Phase 0 spike (MacBook, macOS 26.4, Tauri 2, release build)

| Metric | Result | Budget | |
|---|---|---|---|
| Cold start, exec → first paint (median of 10) | 283 ms | < 300 ms | ✅ barely |
| Cold start, worst of 10 (first launch after build) | 892 ms | — | disk cache cold |
| Cold start, in-process (Rust main → first paint) | 269 ms | — | nearly all of it is webview + JS |
| Idle memory, RSS app + 3 WebKit helpers | 190 MB | < 150 MB | ❌ see note |
| Idle memory, physical footprint: app / WebKit GPU | 27 MB / 59 MB | — | WebContent not readable without sudo |
| Bundle (Fabio.app) | 5.8 MB | < 20 MB | ✅ |
| JS bundle | 222 KB (70 KB gzip) | — | React + nothing else yet |

Notes:
- RSS double-counts shared WebKit framework pages, so 190 MB overstates the real
  cost. Physical footprint is the honest metric; the bench needs a way to read the
  WebContent process's footprint before this budget can be judged fairly.
- Almost all startup is in the webview (269 of 283 ms), not Rust. Levers if it
  regresses: smaller JS (Preact, code-split the editor/grid), show the window
  only after first paint, inline critical CSS.

## 2026-09-26 — Phase 1 (browse UI, virtualized grid, keychain, dialog plugin)

| Metric | Result | Budget | |
|---|---|---|---|
| Cold start, median of 10 | 283 ms (max 310) | < 300 ms | ✅ unchanged from Phase 0 |
| Idle memory, RSS | 188 MB | < 150 MB | ❌ same caveat as above |
| Bundle | 6.3 MB | < 20 MB | ✅ |
| Open `track` first page (200 rows), in-app | 1.1 ms | < 100 ms | ✅ |

## 2026-09-26 — Phase 4 (agent panel, MCP server), first run of `dev/check.sh --budgets`

| Metric | Result | Budget | |
|---|---|---|---|
| Cold start, median of 15 | 283 ms (max 284) | < 300 ms | ✅ unchanged since Phase 1 |
| Cold start, median of 10 right after a build | 300 ms (max 317) | < 300 ms | on the line: first runs after a build are slower |
| Idle memory, RSS | 193 MB | < 150 MB | reported, not enforced (same caveat) |
| Bundle | 7.8 MB | < 20 MB | ✅ |
| JS: main / editor (lazy) / formatter (lazy) | 306 KB / 370 KB / 262 KB | — | main was 222 KB in Phase 0 |

The check now fails on start > 300 ms or bundle > 20 MB. A run right after a
build can land on 300 ms; rerun before treating it as a regression.

## 2026-09-26 — v0.1 features (switcher, ⌘K, editing, menu, window state)

| Metric | Result | Budget | |
|---|---|---|---|
| Cold start, median of 15 | 284 ms (max 300) | < 300 ms | ✅ unchanged |
| Idle memory, RSS | 205 MB | < 150 MB | reported only |
| Bundle | 8.2 MB | < 20 MB | ✅ |
| JS main | 341 KB | — | up from 306 KB; lazy-load the dialogs next |

## 2026-09-26 — speed pass: lazy views, window shown painted, scroll measured

| Metric | Result | Budget | |
|---|---|---|---|
| Cold start, median of 20 (two runs) | 284 / 267 ms | < 300 ms | ✅ |
| JS main | 262 KB | — | was 341 KB; views and dialogs load on first use |
| Scroll 100k rows × 12 columns, fast fling, 3 s | p50 17.0 ms, p95 18.0 ms, max 20 ms, 0% dropped, 0 blank | 60 fps | ✅ first measurement |
| Bundle | 8.3 MB | < 20 MB | ✅ |

Scrolling runs at the display's 60 Hz with no dropped frames, so a canvas/WebGL
grid would buy nothing measurable; the DOM grid stays. The benchmark uses
synthetic rows (`FABIO_BENCH=scroll`), so it measures rendering, not page loads
from a database.

The window now starts hidden and is shown once the page has rendered (no white
frame, even in dark mode). A hidden window gets no animation frames, so the
first attempt (show on first paint) waited for the 1.5 s safety net: 1717 ms.
The budget check caught it.
