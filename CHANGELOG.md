# Changelog

## [Unreleased]

### Added
- Expanded `README.md` with the current helper-script driven workflow, data export guidance, and troubleshooting notes (BTF parsing, CLI permissions).
- Summarized loader/CLI behaviour so new contributors understand how `scripts/run.sh` orchestrates build/load/dump steps.

### Changed
- `scripts/run.sh load` now normalizes bpffs permissions (directory `0755`, map `0644`) and `dump` re-executes the CLI via sudo, fixing the previous `EPERM`/`EBADF` issues when reading pinned maps.
- Rust CLI now links directly against libbpf for map operations, eliminating the brittle manual `SYS_bpf` attr layouts.

### Fixed
- Addressed CO-RE load failures by ensuring the loader prints/uses the correct BTF path and by documenting the debugging process in `docs/ERROR_REPORT.md`.
