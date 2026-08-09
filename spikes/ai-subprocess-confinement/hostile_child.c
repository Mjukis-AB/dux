#include <errno.h>
#include <fcntl.h>
#include <stddef.h>
#include <string.h>
#include <unistd.h>

enum {
    READ_SUCCEEDED = 42,
    READ_DENIED = 23,
    PROBE_FAILED = 24,
};

static int read_exact(int descriptor, char *output, size_t capacity) {
    size_t used = 0;
    while (used < capacity) {
        ssize_t count = read(descriptor, output + used, capacity - used);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count < 0) {
            return -1;
        }
        if (count == 0) {
            break;
        }
        used += (size_t)count;
    }
    return (int)used;
}

int main(int argument_count, char **arguments) {
    if (argument_count != 2 || arguments[1][0] != '/') {
        return PROBE_FAILED;
    }

    int descriptor = open(arguments[1], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (descriptor < 0) {
        return errno == EACCES || errno == EPERM ? READ_DENIED : PROBE_FAILED;
    }

    static const char expected[] = "DUX-CONFINEMENT-CANARY-V1";
    char observed[sizeof(expected)] = {0};
    int count = read_exact(descriptor, observed, sizeof(observed));
    (void)close(descriptor);
    if (count != (int)(sizeof(expected) - 1) ||
        memcmp(observed, expected, sizeof(expected) - 1) != 0) {
        return PROBE_FAILED;
    }
    return READ_SUCCEEDED;
}
