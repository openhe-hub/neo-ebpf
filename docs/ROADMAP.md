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
- Experiment with EDF or hybrid schedulers by deriving deadlines/periods from stats.
- Feed metrics into visualization dashboards or tracing pipelines (e.g., perfetto).
- Introduce alerting/reporting hooks for long-term regression tracking.
