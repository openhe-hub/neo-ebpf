// SPDX-License-Identifier: GPL-2.0
#include "vmlinux.h"
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

struct task_info {
    __u64 runtime_ns;
    __u64 switches;
    __s32 nice;
    __u32 tickets;
    __u64 last_switch_in_ts;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);
    __type(value, struct task_info);
    __uint(max_entries, 10240);
} task_map SEC(".maps");

static __always_inline __u32 nice_to_tickets(__s32 nice)
{
    if (nice < -20)
        nice = -20;
    if (nice > 19)
        nice = 19;

    __u32 base = 100;
    __u32 alpha = 10;
    __s32 scaled = base + alpha * (-nice);
    if (scaled < 10)
        return 10;
    return scaled;
}

static __always_inline struct task_info *get_task_info(__u32 pid)
{
    struct task_info *info = bpf_map_lookup_elem(&task_map, &pid);
    if (!info) {
        struct task_info zero = {};
        bpf_map_update_elem(&task_map, &pid, &zero, BPF_ANY);
        info = bpf_map_lookup_elem(&task_map, &pid);
    }

    return info;
}

SEC("tracepoint/sched/sched_switch")
int handle_sched_switch(struct trace_event_raw_sched_switch *ctx)
{
    __u64 now = bpf_ktime_get_ns();

    __u32 prev_pid = ctx->prev_pid;
    if (prev_pid) {
        struct task_info *prev_info = get_task_info(prev_pid);
        if (prev_info) {
            if (prev_info->last_switch_in_ts && now > prev_info->last_switch_in_ts) {
                prev_info->runtime_ns += now - prev_info->last_switch_in_ts;
            }
            prev_info->switches += 1;
        }
    }

    __u32 next_pid = ctx->next_pid;
    if (next_pid) {
        struct task_info *next_info = get_task_info(next_pid);
        if (next_info) {
            next_info->last_switch_in_ts = now;
            __s32 nice = ctx->next_prio - 120;
            next_info->nice = nice;
            next_info->tickets = nice_to_tickets(nice);
        }
    }

    return 0;
}

char LICENSE[] SEC("license") = "GPL";
