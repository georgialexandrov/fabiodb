#!/usr/bin/env python3
"""Measure the release build against the PLAN.md budgets.

    python3 bench/startup.py [runs]

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


if __name__ == "__main__":
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 10
    wall, inner = startup(runs)
    mem, helpers = idle_memory()
    print(f"cold start (wall, exec→first paint): median {statistics.median(wall):.0f} ms, max {max(wall):.0f} ms  [budget 300]")
    print(f"cold start (in-process, main→first paint): median {statistics.median(inner):.0f} ms")
    print(f"idle memory (RSS, app + {helpers} WebKit helpers): {mem:.0f} MB  [budget 150]")
    print(f"bundle: {du_mb(APP):.1f} MB  [budget 20]")
