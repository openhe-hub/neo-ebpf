// Simple CPU bound workload for exercising the scheduler tracing code.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

static void usage(const char *prog)
{
    fprintf(stderr, "Usage: %s [--nice N] [--duration SECONDS]\n", prog);
}

static double seconds_since(const struct timespec *start, const struct timespec *now)
{
    double sec = (double)(now->tv_sec - start->tv_sec);
    double nsec = (double)(now->tv_nsec - start->tv_nsec) / 1e9;
    return sec + nsec;
}

int main(int argc, char **argv)
{
    int nice_delta = 0;
    int duration = 5;

    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--nice") == 0) {
            if (i + 1 >= argc) {
                usage(argv[0]);
                return 1;
            }
            nice_delta = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--duration") == 0) {
            if (i + 1 >= argc) {
                usage(argv[0]);
                return 1;
            }
            duration = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--help") == 0) {
            usage(argv[0]);
            return 0;
        } else {
            fprintf(stderr, "Unknown argument: %s\n", argv[i]);
            usage(argv[0]);
            return 1;
        }
    }

    if (setpriority(PRIO_PROCESS, 0, nice_delta) == -1) {
        perror("setpriority");
        return 1;
    }

    fprintf(stdout, "Running CPU-bound workload for %d seconds at nice %d (pid=%d)\n",
            duration, nice_delta, getpid());
    fflush(stdout);

    struct timespec start, now;
    if (clock_gettime(CLOCK_MONOTONIC, &start) != 0) {
        perror("clock_gettime");
        return 1;
    }

    volatile unsigned long accumulator = 0;
    do {
        for (int i = 0; i < 1000000; ++i) {
            accumulator += (unsigned long)i;
        }

        if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
            perror("clock_gettime");
            return 1;
        }
    } while (seconds_since(&start, &now) < (double)duration);

    fprintf(stdout, "Workload complete (acc=%lu)\n", accumulator);
    return 0;
}
