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
