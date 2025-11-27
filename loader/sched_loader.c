#include <errno.h>
#include <getopt.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>

#include <bpf/libbpf.h>

#ifndef LIBBPF_STRICT_AUTO
#define LIBBPF_STRICT_AUTO LIBBPF_STRICT_ALL
#endif

struct config {
    const char *obj_path;
    const char *prog_pin;
    const char *map_pin;
    const char *link_pin;
    const char *trace_point;
    const char *btf_path;
};

static void usage(const char *prog)
{
    fprintf(stderr,
            "Usage: %s --obj PATH --prog-pin PATH --map-pin PATH --link-pin PATH "
            "[--trace category:name] [--btf PATH]\n",
            prog);
}

static int bump_memlock_rlimit(void)
{
    struct rlimit rl = {
        .rlim_cur = RLIM_INFINITY,
        .rlim_max = RLIM_INFINITY,
    };

    return setrlimit(RLIMIT_MEMLOCK, &rl);
}

static int repin_map(struct bpf_map *map, const char *pin_path)
{
    int err = bpf_map__unpin(map, pin_path);
    if (err && err != -ENOENT)
        return err;
    return bpf_map__pin(map, pin_path);
}

static int repin_program(struct bpf_program *prog, const char *pin_path)
{
    int err = bpf_program__unpin(prog, pin_path);
    if (err && err != -ENOENT)
        return err;
    return bpf_program__pin(prog, pin_path);
}

static int attach_tracepoint(struct bpf_program *prog, const char *trace, const char *link_pin)
{
    char *trace_copy = NULL;
    char *sep = NULL;
    char *category = NULL;
    char *name = NULL;
    struct bpf_link *link = NULL;
    int err = 0;

    trace_copy = strdup(trace);
    if (!trace_copy)
        return -ENOMEM;

    sep = strchr(trace_copy, ':');
    if (!sep) {
        err = -EINVAL;
        goto out;
    }

    *sep = '\0';
    category = trace_copy;
    name = sep + 1;

    link = bpf_program__attach_tracepoint(prog, category, name);
    err = libbpf_get_error(link);
    if (err) {
        link = NULL;
        goto out;
    }

    if (unlink(link_pin) && errno != ENOENT) {
        err = -errno;
        goto out;
    }

    err = bpf_link__pin(link, link_pin);
    if (err)
        goto out;

out:
    if (link)
        bpf_link__destroy(link);
    free(trace_copy);
    return err;
}

int main(int argc, char **argv)
{
    static const struct option opts[] = {
        {"obj", required_argument, NULL, 'o'},
        {"prog-pin", required_argument, NULL, 'p'},
        {"map-pin", required_argument, NULL, 'm'},
        {"link-pin", required_argument, NULL, 'l'},
        {"trace", required_argument, NULL, 't'},
        {"btf", required_argument, NULL, 'b'},
        {"help", no_argument, NULL, 'h'},
        {}
    };

    struct config cfg = {
        .trace_point = "sched:sched_switch",
    };
    int opt;
    int err;
    struct bpf_object *obj = NULL;
    struct bpf_program *prog = NULL;
    struct bpf_map *map = NULL;

    while ((opt = getopt_long(argc, argv, "", opts, NULL)) != -1) {
        switch (opt) {
        case 'o':
            cfg.obj_path = optarg;
            break;
        case 'p':
            cfg.prog_pin = optarg;
            break;
        case 'm':
            cfg.map_pin = optarg;
            break;
        case 'l':
            cfg.link_pin = optarg;
            break;
        case 't':
            cfg.trace_point = optarg;
            break;
        case 'b':
            cfg.btf_path = optarg;
            break;
        case 'h':
        default:
            usage(argv[0]);
            return opt == 'h' ? 0 : 1;
        }
    }

    if (!cfg.obj_path || !cfg.prog_pin || !cfg.map_pin || !cfg.link_pin) {
        usage(argv[0]);
        return 1;
    }

    libbpf_set_strict_mode(LIBBPF_STRICT_AUTO);
    if (bump_memlock_rlimit()) {
        perror("setrlimit");
        return 1;
    }

    struct bpf_object_open_opts open_opts = {
        .sz = sizeof(open_opts),
    };
    if (cfg.btf_path) {
        printf("cfg.btf_path=%s\n", cfg.btf_path);
        open_opts.btf_custom_path = cfg.btf_path;
    } else {
        printf("cfg.btf_path=(null)\n");
    }

    obj = bpf_object__open_file(cfg.obj_path, &open_opts);
    err = libbpf_get_error(obj);
    if (err) {
        fprintf(stderr, "Failed to open %s: %s\n", cfg.obj_path, strerror(-err));
        obj = NULL;
        goto cleanup;
    }

    prog = bpf_object__find_program_by_name(obj, "handle_sched_switch");
    if (!prog) {
        fprintf(stderr, "Program handle_sched_switch not found in %s\n", cfg.obj_path);
        err = -ENOENT;
        goto cleanup;
    }

    map = bpf_object__find_map_by_name(obj, "task_map");
    if (!map) {
        fprintf(stderr, "Map task_map not found in %s\n", cfg.obj_path);
        err = -ENOENT;
        goto cleanup;
    }

    err = bpf_object__load(obj);
    if (err) {
        fprintf(stderr, "Failed to load %s: %s\n", cfg.obj_path, strerror(-err));
        goto cleanup;
    }

    err = repin_map(map, cfg.map_pin);
    if (err) {
        fprintf(stderr, "Failed to pin map at %s: %s\n", cfg.map_pin, strerror(-err));
        goto cleanup;
    }

    err = repin_program(prog, cfg.prog_pin);
    if (err) {
        fprintf(stderr, "Failed to pin program at %s: %s\n", cfg.prog_pin, strerror(-err));
        goto cleanup;
    }

    err = attach_tracepoint(prog, cfg.trace_point, cfg.link_pin);
    if (err) {
        fprintf(stderr, "Failed to attach %s: %s\n", cfg.trace_point, strerror(-err));
        goto cleanup;
    }

    printf("Loaded %s, pinned prog=%s map=%s link=%s\n",
           cfg.obj_path, cfg.prog_pin, cfg.map_pin, cfg.link_pin);

cleanup:
    if (obj)
        bpf_object__close(obj);

    return err ? 1 : 0;
}
