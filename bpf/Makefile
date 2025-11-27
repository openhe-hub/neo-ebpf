BPF_CLANG ?= clang
BPF_CFLAGS := -O2 -g -target bpf -D__TARGET_ARCH_x86 -Wall -Werror -Wno-unused-value -Wno-pointer-sign

all: sched_lottery.bpf.o

vmlinux.h: FORCE
	bpftool btf dump file /sys/kernel/btf/vmlinux format c > vmlinux.h

sched_lottery.bpf.o: sched_lottery.bpf.c vmlinux.h
	$(BPF_CLANG) $(BPF_CFLAGS) -I. -c $< -o $@

clean:
	rm -f sched_lottery.bpf.o

FORCE:

.PHONY: all clean FORCE
