import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// Benchmarks replace the app (FABIO_BENCH, see bench/startup.py).
invoke<string | null>("bench_mode").then(async (mode) => {
  if (mode !== "scroll") return;
  const { ScrollBench } = await import("./ScrollBench");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<ScrollBench />);
});

// Rendered: the shell shows the window (hidden until now, so no white frame).
// Then, once it has painted, report for the startup measurement.
setTimeout(() =>
  invoke("app_rendered").then(() => requestAnimationFrame(() => requestAnimationFrame(() => invoke("app_ready")))),
);
