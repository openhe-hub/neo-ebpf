CC ?= clang
CFLAGS ?= -O2 -g -Wall -Wextra -Werror
LIBBPF_CFLAGS ?= $(shell pkg-config --cflags libbpf 2>/dev/null)
LIBBPF_LDLIBS ?= $(shell pkg-config --libs libbpf 2>/dev/null)

ifeq ($(strip $(LIBBPF_CFLAGS)),)
LIBBPF_CFLAGS := -I/usr/include/bpf
endif

ifeq ($(strip $(LIBBPF_LDLIBS)),)
LIBBPF_LDLIBS := -lbpf -lelf -lz
endif

LDLIBS ?= $(LIBBPF_LDLIBS)

OUTPUT ?= sched_loader
SRC := sched_loader.c

all: $(OUTPUT)

$(OUTPUT): $(SRC)
	$(CC) $(CFLAGS) $(LIBBPF_CFLAGS) $< -o $@ $(LDLIBS)

clean:
	rm -f $(OUTPUT)

.PHONY: all clean
