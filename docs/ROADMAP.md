# Project Roadmap

## Milestone 1 — Baseline tracer & dumper
- Trace `sched_switch` via eBPF and collect per-task runtime/switch counts.
- Pin the task map and expose a simple Rust CLI for periodic dumps/CSV capture.
- Provide helper workloads for easy validation.

## Milestone 2 — User-space lottery simulation
- Extend the Rust runner to run a lottery draw using the recorded ticket counts.
- Produce rankings or scheduling order suggestions based on collected stats.
- Emit richer telemetry (per-cgroup summaries, rolling averages).

## Milestone 3 — Advanced scheduling research
- Experiment with EDF or hybrid schedulers by deriving deadlines/periods from stats.
- Feed metrics into visualization dashboards or tracing pipelines (e.g., perfetto).
- Introduce alerting/reporting hooks for long-term regression tracking.
