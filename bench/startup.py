#!/usr/bin/env python3
"""Measure the release build against the PLAN.md budgets.

    python3 bench/startup.py [runs] [--bundle-only]

Exits 1 when cold start or bundle size is over budget.

Cold start = process exec → frontend's second animation frame (first paint),
measured from outside (wall clock) and inside (Rust main → app_ready).
Idle memory = main process + the WebKit helper processes it spawned, as RSS.
"""
import os, statistics, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(ROOT, "target/release/bundle/macos/Fabio.app")
BIN = os.path.join(APP, "Contents/MacOS/fabio")


def webkit_pids():
    out = subprocess.run(["pgrep", "-f", "com.apple.WebKit"], capture_output=True, text=True).stdout
    return {int(p) for p in out.split()}


def rss_mb(pids):
    if not pids:
        return 0.0
    out = subprocess.run(["ps", "-o", "rss=", "-p", ",".join(map(str, pids))], capture_output=True, text=True).stdout
    return sum(int(x) for x in out.split()) / 1024


def startup(runs):
    wall, inner = [], []
    env = {**os.environ, "FABIO_EXIT_ON_READY": "1"}
    # One unmeasured launch first: right after a build the disk cache is cold
    # and the first start is 3-4x slower than any real-world launch.
    subprocess.run([BIN], env=env, capture_output=True, timeout=30)
    for _ in range(runs):
        t = time.perf_counter()
        out = subprocess.run([BIN], env=env, capture_output=True, text=True, timeout=30).stdout
        wall.append((time.perf_counter() - t) * 1000)
        inner.append(float(out.split("ready_ms=")[1].split()[0]))
    return wall, inner


def idle_memory():
    before = webkit_pids()
    proc = subprocess.Popen([BIN])
    time.sleep(4)
    helpers = webkit_pids() - before
    mb = rss_mb({proc.pid} | helpers)
    proc.terminate()
    proc.wait()
    return mb, len(helpers)


def du_mb(path):
    return int(subprocess.run(["du", "-sk", path], capture_output=True, text=True).stdout.split()[0]) / 1024


def scroll():
    """Frame times scrolling 100k rows (FABIO_BENCH=scroll), as a dict of floats."""
    env = {**os.environ, "FABIO_BENCH": "scroll"}
    out = subprocess.run([BIN], env=env, capture_output=True, text=True, timeout=60).stdout
    line = next(l for l in out.splitlines() if l.startswith("scroll_frames="))
    return {k: float(v) for k, v in (pair.split("=") for pair in line.split())}


START_BUDGET_MS = 300
# 60 fps: at most 1% of frames over 25 ms (a visibly dropped frame), no blank rows.
SCROLL_DROPPED_BUDGET_PCT = 1.0
BUNDLE_BUDGET_MB = 20


if __name__ == "__main__":
    # --bundle-only: CI machines aren't this Mac, so only the size is comparable there.
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    bundle_only = "--bundle-only" in sys.argv
    over = []

    if not bundle_only:
        runs = int(args[0]) if args else 10
        wall, inner = startup(runs)
        mem, helpers = idle_memory()
        start = statistics.median(wall)
        print(f"cold start (wall, exec→first paint): median {start:.0f} ms, max {max(wall):.0f} ms  [budget {START_BUDGET_MS}]")
        print(f"cold start (in-process, main→first paint): median {statistics.median(inner):.0f} ms")
        # Reported, not enforced: RSS overcounts shared WebKit pages (BENCHMARKS.md).
        print(f"idle memory (RSS, app + {helpers} WebKit helpers): {mem:.0f} MB  [budget 150, not enforced yet]")
        if start > START_BUDGET_MS:
            over.append(f"cold start {start:.0f} ms > {START_BUDGET_MS} ms")
        s = scroll()
        print(
            f"scroll 100k rows: p50 {s['p50_ms']:.1f} ms, p95 {s['p95_ms']:.1f} ms, max {s['max_ms']:.0f} ms, "
            f"dropped {s['dropped_pct']:.1f}%, blank frames {s['blank_frames']:.0f}  [budget 60 fps: ≤ {SCROLL_DROPPED_BUDGET_PCT}% dropped, no blank]"
        )
        if s["dropped_pct"] > SCROLL_DROPPED_BUDGET_PCT or s["blank_frames"] > 0:
            over.append(f"scroll dropped {s['dropped_pct']:.1f}% of frames, {s['blank_frames']:.0f} blank")

    bundle = du_mb(APP)
    print(f"bundle: {bundle:.1f} MB  [budget {BUNDLE_BUDGET_MB}]")
    if bundle > BUNDLE_BUDGET_MB:
        over.append(f"bundle {bundle:.1f} MB > {BUNDLE_BUDGET_MB} MB")

    if over:
        print("over budget: " + "; ".join(over), file=sys.stderr)
        sys.exit(1)
