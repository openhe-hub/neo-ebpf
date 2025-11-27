#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BPF_DIR="$ROOT_DIR/bpf"
LOADER_DIR="$ROOT_DIR/loader"
LOADER_BIN="$LOADER_DIR/sched_loader"
RUST_DIR="$ROOT_DIR/rust-runner"
TEST_SRC="$ROOT_DIR/tests/cpu_bound.c"
TEST_BIN="$ROOT_DIR/tests/cpu_bound"
MAP_PIN=${MAP_PIN:-/sys/fs/bpf/task_map}
PROG_PIN=${PROG_PIN:-/sys/fs/bpf/sched_lottery}
LINK_PIN_DEFAULT="${PROG_PIN}_link"
LINK_PIN=${LINK_PIN:-$LINK_PIN_DEFAULT}
BTF_PATH=${BTF_PATH:-/sys/kernel/btf/vmlinux}

LIBBPF_PC_DIR=${LIBBPF_PC_DIR:-/usr/lib64/pkgconfig}
LIBBPF_LIB_DIR=${LIBBPF_LIB_DIR:-/usr/lib64}

if [ -d "$LIBBPF_PC_DIR" ]; then
    export PKG_CONFIG_PATH="$LIBBPF_PC_DIR${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi

if [ -d "$LIBBPF_LIB_DIR" ]; then
    export LD_LIBRARY_PATH="$LIBBPF_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing dependency: $1" >&2
        exit 1
    fi
}

build_bpf() {
    require_cmd clang
    require_cmd bpftool
    echo "[+] Building BPF object"
    (cd "$BPF_DIR" && make)
}

build_loader() {
    require_cmd clang
    require_cmd make
    echo "[+] Building loader"
    (cd "$LOADER_DIR" && make)
}

build_rust() {
    require_cmd cargo
    echo "[+] Building Rust runner"
    (cd "$RUST_DIR" && cargo build --release)
}

build_workload() {
    require_cmd gcc
    if [ ! -x "$TEST_BIN" ] || [ "$TEST_SRC" -nt "$TEST_BIN" ]; then
        echo "[+] Compiling test workload"
        gcc -O2 "$TEST_SRC" -o "$TEST_BIN"
    fi
}

run_as_root() {
    if [ "${EUID}" -ne 0 ]; then
        sudo PKG_CONFIG_PATH="/usr/lib64/pkgconfig:$PKG_CONFIG_PATH" LD_LIBRARY_PATH="/usr/lib64:$LD_LIBRARY_PATH" "$@"
    else
        "$@"
    fi
}

remove_path() {
    local path="$1"
    if [ -e "$path" ]; then
        echo "[+] Removing $path"
        if [ "${EUID}" -ne 0 ]; then
            sudo rm -rf "$path"
        else
            rm -rf "$path"
        fi
    fi
}

ensure_dir() {
    local dir="$1"
    if [ ! -d "$dir" ]; then
        echo "[+] Creating directory $dir"
        if [ "${EUID}" -ne 0 ];	then
            sudo mkdir -p "$dir"
        else
            mkdir -p "$dir"
        fi
    fi
}

load_bpf() {
    build_bpf
    build_loader
    echo "[+] Cleaning up any existing pins"
    remove_path "$LINK_PIN"
    remove_path "$PROG_PIN"
    remove_path "$MAP_PIN"
    ensure_dir "$(dirname "$MAP_PIN")"
    ensure_dir "$(dirname "$PROG_PIN")"
    ensure_dir "$(dirname "$LINK_PIN")"
    run_as_root chmod 755 "$(dirname "$MAP_PIN")"
    echo "[+] Loading and attaching via loader"
    local loader_cmd=("$LOADER_BIN" \
        --obj "$BPF_DIR/sched_lottery.bpf.o" \
        --prog-pin "$PROG_PIN" \
        --map-pin "$MAP_PIN" \
        --link-pin "$LINK_PIN")
    if [ -r "$BTF_PATH" ]; then
        loader_cmd+=("--btf" "$BTF_PATH")
    else
        echo "[!] BTF file $BTF_PATH not readable; relying on embedded BTF" >&2
    fi
    run_as_root "${loader_cmd[@]}"
    echo "[+] Relaxing map permissions for user access"
    run_as_root chmod 644 "$MAP_PIN"
}

unload_bpf() {
    echo "[+] Removing pinned objects"
    remove_path "$LINK_PIN"
    remove_path "$PROG_PIN"
    remove_path "$MAP_PIN"
}

dump_stats() {
    build_rust
    local bin="$RUST_DIR/target/release/rust-runner"
    if [ ! -x "$bin" ]; then
        echo "Runner binary missing: $bin" >&2
        exit 1
    fi
    echo "[+] Running CLI"
    if [ "${EUID}" -ne 0 ]; then
        run_as_root "$bin" dump --map "$MAP_PIN" "$@"
    else
        "$bin" dump --map "$MAP_PIN" "$@"
    fi
}

run_workload() {
    build_workload
    echo "[+] Launching workload: $TEST_BIN $*"
    "$TEST_BIN" "$@"
}

usage() {
    cat <<USAGE
Usage: $0 <command> [args]

Commands:
  build             Build BPF object, Rust runner, and workload helper
  load              Load + attach the BPF program (requires root/sudo)
  unload            Detach and remove pinned program/map
  dump [args]       Run the Rust CLI (extra args passed through to 'dump')
  workload [args]   Run the CPU workload helper (defaults see tests/cpu_bound.c)
  help              Show this help

Environment overrides:
  MAP_PIN    (default: /sys/fs/bpf/task_map)
  PROG_PIN   (default: /sys/fs/bpf/sched_lottery)
USAGE
}

CMD=${1:-help}
shift || true

case "$CMD" in
    build)
        build_bpf
        build_loader
        build_rust
        build_workload
        ;;
    load)
        load_bpf
        ;;
    unload)
        unload_bpf
        ;;
    dump)
        dump_stats "$@"
        ;;
    workload)
        run_workload "$@"
        ;;
    help|--help|-h)
        usage
        ;;
    *)
        echo "Unknown command: $CMD" >&2
        usage
        exit 1
        ;;
 esac
