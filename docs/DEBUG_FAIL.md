# eBPF Loader Failure Report

## 概要

`loader/sched_loader`（基于 libbpf 的 CO‑RE loader）在加载 `bpf/sched_lottery.bpf.o` 时始终输出：

```
libbpf: failed to parse target BTF: -22
libbpf: failed to perform CO-RE relocations: -22
libbpf: failed to load object '/media/openhe/E盘/program/rust/p3-ebpf/bpf/sched_lottery.bpf.o'
Failed to load /media/openhe/E盘/program/rust/p3-ebpf/bpf/sched_lottery.bpf.o: Invalid argument
```

即使系统安装了 libbpf 1.7.0，`pkg-config --modversion libbpf` 也确认版本正确，CO‑RE 仍因解析 `/sys/kernel/btf/vmlinux` 失败而中断。

## 相关组件

### loader/sched_loader.c（节选）

```c
struct config {
    const char *obj_path;
    const char *prog_pin;
    const char *map_pin;
    const char *link_pin;
    const char *trace_point;
    const char *btf_path;
};

int main(int argc, char **argv)
{
    struct bpf_object_open_opts open_opts = {
        .sz = sizeof(open_opts),
    };
    open_opts.btf_custom_path = cfg.btf_path;

    obj = bpf_object__open_file(cfg.obj_path, &open_opts);
    ...
    prog = bpf_object__find_program_by_name(obj, "handle_sched_switch");
    map = bpf_object__find_map_by_name(obj, "task_map");

    err = bpf_object__load(obj);   // <-- 这里触发 failed to parse target BTF
    ...
    link = bpf_program__attach_tracepoint(prog, category, name);
    bpf_link__pin(link, link_pin);
}
```

### scripts/run.sh（load 路径）

```bash
LIBBPF_PC_DIR=${LIBBPF_PC_DIR:-/usr/lib64/pkgconfig}
LIBBPF_LIB_DIR=${LIBBPF_LIB_DIR:-/usr/lib64}

if [ -d "$LIBBPF_PC_DIR" ]; then
    export PKG_CONFIG_PATH="$LIBBPF_PC_DIR${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi
if [ -d "$LIBBPF_LIB_DIR" ]; then
    export LD_LIBRARY_PATH="$LIBBPF_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

load_bpf() {
    build_bpf
    build_loader
    ...
    run_as_root "$LOADER_BIN" \
        --obj "$BPF_DIR/sched_lottery.bpf.o" \
        --prog-pin "$PROG_PIN" \
        --map-pin "$MAP_PIN" \
        --link-pin "$LINK_PIN" \
        --btf "$BTF_PATH"
}
```

## 已确认事实

1. `/sys/kernel/btf/vmlinux` 存在且可读（约 6MB）。
2. libbpf 1.7.0 已安装（`pkg-config --modversion libbpf` 返回 1.7.0）。
3. loader 可成功编译并运行，但在 `bpf_object__load()` 阶段解析 BTF 失败（errno -22）。
4. 若不使用 loader，而是直接 `bpftool prog load`，不会遇到该问题（因为不依赖 CO‑RE）。

## 待排查

- 目标内核是否与构建使用的 BTF 不匹配，导致 `bpf_object__load()` 无法完成 CO‑RE 解析。
- 是否需要指定其它 BTF（例如 `/sys/kernel/btf/vmlinux-<version>`）或在 loader 中禁用 CO‑RE。
- libbpf 1.7.0 的运行时路径、`LD_LIBRARY_PATH` 等是否在 sudo 环境下完全生效。

请高级工程师基于以上信息诊断该问题。如需更多环境日志或 `perf_event_open` 的替代方案，请告知。review
