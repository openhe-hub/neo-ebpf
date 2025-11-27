# 调试纪录：`sched_loader` 解析目标 BTF 失败

## 现象与复现方式

通过项目脚本加载 eBPF 程序：

```bash
./scripts/run.sh load
```

在「清理旧的 pin」「重新编译 loader」后，`sched_loader` 始终在 `bpf_object__load()` 阶段报错：

```
libbpf: failed to parse target BTF: -22
libbpf: failed to perform CO-RE relocations: -22
libbpf: failed to load object '/media/openhe/E盘/program/rust/p3-ebpf/bpf/sched_lottery.bpf.o'
Failed to load /media/openhe/E盘/program/rust/p3-ebpf/bpf/sched_lottery.bpf.o: Invalid argument
```

同一份 `sched_lottery.bpf.o` 可以用 `bpftool prog load` 正常加载，说明 BPF 对象自身没有问题。失败发生在 loader 解析「目标 BTF」期间。

## 关键代码（两处）

1. **脚本传参与环境**：`scripts/run.sh` 在 `load_bpf()` 中构造 loader 命令并强制传递 `--btf /sys/kernel/btf/vmlinux`，同时在 `run_as_root()` 里用 `sudo` 执行：

```bash
local loader_cmd=("$LOADER_BIN" \
    --obj "$BPF_DIR/sched_lottery.bpf.o" \
    --prog-pin "$PROG_PIN" \
    --map-pin "$MAP_PIN" \
    --link-pin "$LINK_PIN")
if [ -r "$BTF_PATH" ]; then
    loader_cmd+=("--btf" "$BTF_PATH")
fi
run_as_root "${loader_cmd[@]}"
```

```bash
run_as_root() {
    if [ "${EUID}" -ne 0 ]; then
        sudo PKG_CONFIG_PATH="/usr/lib64/pkgconfig:$PKG_CONFIG_PATH" \
            LD_LIBRARY_PATH="/usr/lib64:$LD_LIBRARY_PATH" "$@"
    else
        "$@"
    fi
}
```

2. **loader 里的 `btf_custom_path` 设置**：`loader/sched_loader.c` 在 `struct bpf_object_open_opts` 中直接写入 `cfg.btf_path`，不做空指针判断，随后调用 `bpf_object__open_file()`：

```c
struct bpf_object_open_opts open_opts = {
    .sz = sizeof(open_opts),
};
open_opts.btf_custom_path = cfg.btf_path;

obj = bpf_object__open_file(cfg.obj_path, &open_opts);
```

这意味着只要脚本传了 `--btf`，libbpf 就会**强制**解析对应文件，再进入 CO-RE Relocation。

## 分析：为什么会是 `failed to parse target BTF`

`libbpf` 调用链为 `bpf_object__load()` → `bpf_object__relocate_core()` → `btf__parse()`。
`btf__parse()` 返回 `-EINVAL`（即 `-22`）只有以下可能：

1. **Loader 实际链接到的 libbpf 版本/路径与期望不同**。  
   `bpftool` 自带的 libbpf 能解析当前内核 BTF，而 loader（由 `sudo` 调用）可能仍然链接到较老的系统 libbpf，导致无法解析 `/sys/kernel/btf/vmlinux`。`sudo` 默认丢弃 `LD_LIBRARY_PATH`，脚本中临时设置的路径可能没生效。

2. **传入的 BTF 路径不是合法的 BTF 文件**。  
   例如指到了目录或空文件，或者在解析参数时 `cfg.btf_path` 被污染成随机指针，libbpf 自然无法解析。

3. **目标 BTF 文件本身损坏或包含当前 libbpf 未支持的类型扩展**。  
   需要用 `bpftool btf dump file /sys/kernel/btf/vmlinux | head` 直接验证。

以上三点与 “CO-RE 匹配不上” 不同：这里在「读取目标 BTF」这一步就失败了，说明 loader 根本没有机会完成字段匹配。

## 建议的排查步骤

1. **确认 loader 运行时链接的 libbpf 版本**  
   - `ldd loader/sched_loader | grep libbpf`  
   - `sudo env LD_DEBUG=libs ./loader/sched_loader --help 2>&1 | grep -i libbpf`
   如果看到的是旧版本（如 0.5.0），需调整 `sudo` 环境或直接把 1.7.0 安装到系统标准路径。

2. **验证 BTF 文件本身**  
   - `sudo bpftool btf dump file /sys/kernel/btf/vmlinux | head`  
   若此命令也失败，需重新生成/安装匹配当前内核的 BTF。

3. **检查 `--btf` 参数解析**  
   - 在 loader 中仅当 `cfg.btf_path != NULL` 时再设置 `btf_custom_path`，并打印实际路径。  
   - 试着去掉 `--btf`（让 libbpf 自动寻找内核 BTF）确认差异。

4. **最小 reproducer**  
   写一个只调用 `btf__parse("/sys/kernel/btf/vmlinux")` 的小程序，以 root 身份运行，验证是否同样返回 -22。

只要确定 loader 与 bpftool 使用同一版 libbpf，并且 `/sys/kernel/btf/vmlinux` 能被 `btf__parse()` 正常解析，CO-RE 加载流程就能继续往下执行。

