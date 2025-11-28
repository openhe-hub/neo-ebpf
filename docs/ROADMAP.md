# Project Roadmap

## Milestone 1 — Baseline tracer & dumper
- Trace `sched_switch` via eBPF and collect per-task runtime/switch counts.
- Pin the task map and expose a simple Rust CLI for periodic dumps/CSV capture.
- Provide helper workloads for easy validation.

## Milestone 2 — User-space lottery simulation
- ✅ Extend the Rust runner to run lottery draws using the recorded ticket counts (see `--simulate-draws`, `--seed`, `--top`).
- ✅ Produce live rankings/scheduling order hints based on ticket share and rolling runtime deltas.
- ⏳ Emit richer telemetry such as per-cgroup summaries (rolling stats + CSV enrichments are in place; per-cgroup metadata pending kernel-support work).

## Milestone 3 — Advanced scheduling research
- ✅ Experiment with EDF or hybrid schedulers by deriving deadlines/periods from stats (CLI now reports heuristic periods, lateness, utilisation).
- ✅ Feed metrics into visualization dashboards or tracing pipelines (NDJSON + Chrome trace exports land under `--json-output/--trace-output`).
- ✅ Introduce alerting/reporting hooks (deadline warnings + EDF summary in CLI).
- ⏳ Next ideas: plug the NDJSON stream into Grafana/Loki or perfetto.dev automation, explore EDF vs. lottery hybrid policies, and add long-running regression tests.
