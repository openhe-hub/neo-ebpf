# Changelog

## [Unreleased]

### Added
- Expanded `README.md` with the current helper-script driven workflow, data export guidance, and troubleshooting notes (BTF parsing, CLI permissions).
- Summarized loader/CLI behaviour so new contributors understand how `scripts/run.sh` orchestrates build/load/dump steps.
- Lottery simulation pipeline in `rust-runner`: EWMA-based rolling metrics, ticket-share ranking, CSV enrichments, and optional reproducible lottery draws via `--simulate-draws/--alpha/--top/--seed`.
- EDF-style heuristics in the Rust CLI (period/lateness/utilisation estimates, alerts via `--deadline-warn`, summary table).
- Visualization exports: NDJSON snapshots (`--json-output`) and Chrome trace / Perfetto JSON dumps (`--trace-output`).
- Interactive terminal dashboard (`rust-runner tui` + `./scripts/run.sh tui`) built with ratatui/crossterm, now with live history sparkline and summary panel (avg/worst lateness, utilisation, top lottery candidate).

### Changed
- `scripts/run.sh load` now normalizes bpffs permissions (directory `0755`, map `0644`) and `dump` re-executes the CLI via sudo, fixing the previous `EPERM`/`EBADF` issues when reading pinned maps.
- Rust CLI now links directly against libbpf for map operations, eliminating the brittle manual `SYS_bpf` attr layouts.
- `README.md` quick-start commands now highlight the richer dump options and note that CSVs live under `assets/` by default.

### Fixed
- Addressed CO-RE load failures by ensuring the loader prints/uses the correct BTF path and by documenting the debugging process in `docs/ERROR_REPORT.md`.
