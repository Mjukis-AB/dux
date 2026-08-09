#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <spawn.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <unistd.h>

#if defined(__APPLE__)
#include <sys/sysctl.h>
#endif

enum {
    CHILD_READ_SUCCEEDED = 42,
    CHILD_READ_DENIED = 23,
    SANDBOX_EXEC_UNAVAILABLE = 127,
};

struct lane_result {
    int child_exit_code;
    const char *outcome;
    bool applied;
};

static bool safe_component(char value) {
    return (value >= 'a' && value <= 'z') ||
           (value >= 'A' && value <= 'Z') ||
           (value >= '0' && value <= '9') || value == '.' || value == '_' ||
           value == '-';
}

static void sanitize_fact(char *value, size_t capacity) {
    if (value[0] == '\0') {
        (void)snprintf(value, capacity, "unknown");
        return;
    }
    for (size_t index = 0; value[index] != '\0'; index++) {
        if (!safe_component(value[index])) {
            (void)snprintf(value, capacity, "unknown");
            return;
        }
    }
}

static void platform_facts(
    char *product_version,
    size_t product_capacity,
    char *build_version,
    size_t build_capacity,
    char *architecture,
    size_t architecture_capacity
) {
    struct utsname facts;
    if (uname(&facts) == 0) {
        (void)snprintf(architecture, architecture_capacity, "%s", facts.machine);
    } else {
        (void)snprintf(architecture, architecture_capacity, "unknown");
    }

#if defined(__APPLE__)
    size_t product_size = product_capacity;
    if (sysctlbyname("kern.osproductversion", product_version, &product_size, NULL, 0) != 0) {
        (void)snprintf(product_version, product_capacity, "unknown");
    }
    size_t build_size = build_capacity;
    if (sysctlbyname("kern.osversion", build_version, &build_size, NULL, 0) != 0) {
        (void)snprintf(build_version, build_capacity, "unknown");
    }
#else
    if (uname(&facts) == 0) {
        (void)snprintf(product_version, product_capacity, "%s", facts.release);
    } else {
        (void)snprintf(product_version, product_capacity, "unknown");
    }
    (void)snprintf(build_version, build_capacity, "unknown");
#endif

    sanitize_fact(product_version, product_capacity);
    sanitize_fact(build_version, build_capacity);
    sanitize_fact(architecture, architecture_capacity);
}

static bool join_path(char *output, size_t capacity, const char *root, const char *leaf) {
    int length = snprintf(output, capacity, "%s/%s", root, leaf);
    return length > 0 && (size_t)length < capacity;
}

static bool reviewed_child_path(const char *host_argument, char *output, size_t capacity) {
    if (host_argument[0] != '/' || realpath(host_argument, output) == NULL) {
        return false;
    }
    char *separator = strrchr(output, '/');
    if (separator == NULL) {
        return false;
    }
    size_t prefix = (size_t)(separator - output) + 1;
    static const char child_name[] = "hostile-child";
    if (prefix + sizeof(child_name) > capacity) {
        return false;
    }
    memcpy(output + prefix, child_name, sizeof(child_name));

    struct stat facts;
    return lstat(output, &facts) == 0 && S_ISREG(facts.st_mode) &&
           facts.st_uid == geteuid() && facts.st_nlink == 1 &&
           (facts.st_mode & (S_IWGRP | S_IWOTH)) == 0;
}

static bool write_all(int descriptor, const char *bytes, size_t length) {
    size_t written = 0;
    while (written < length) {
        ssize_t count = write(descriptor, bytes + written, length - written);
        if (count < 0 && errno == EINTR) {
            continue;
        }
        if (count <= 0) {
            return false;
        }
        written += (size_t)count;
    }
    return true;
}

static int normalized_wait_status(int status) {
    if (WIFEXITED(status)) {
        return WEXITSTATUS(status);
    }
    if (WIFSIGNALED(status)) {
        return 128 + WTERMSIG(status);
    }
    return 255;
}

static struct lane_result run_child(
    const char *child,
    const char *sentinel,
    int working_directory,
    bool use_deprecated_sandbox
) {
    char profile[PATH_MAX + 96] = {0};
    char *direct_arguments[] = {(char *)child, (char *)sentinel, NULL};
    char *sandbox_arguments[] = {
        "/usr/bin/sandbox-exec",
        "-p",
        profile,
        (char *)child,
        (char *)sentinel,
        NULL,
    };
    const char *executable = child;
    char **arguments = direct_arguments;
    if (use_deprecated_sandbox) {
#if defined(__APPLE__)
        int length = snprintf(
            profile,
            sizeof(profile),
            "(version 1) (allow default) (deny file-read* (literal \"%s\"))",
            sentinel
        );
        if (length <= 0 || (size_t)length >= sizeof(profile)) {
            return (struct lane_result){126, "probe_failed", false};
        }
        executable = sandbox_arguments[0];
        arguments = sandbox_arguments;
#else
        return (struct lane_result){SANDBOX_EXEC_UNAVAILABLE, "unavailable", false};
#endif
    }

    char *const environment[] = {
        "HOME=/private/var/empty",
        "LANG=C",
        "PATH=/usr/bin:/bin",
        "TMPDIR=/private/tmp",
        NULL,
    };
    posix_spawn_file_actions_t file_actions;
    posix_spawnattr_t attributes;
    if (posix_spawn_file_actions_init(&file_actions) != 0) {
        return (struct lane_result){255, "launch_failed", false};
    }
    if (posix_spawnattr_init(&attributes) != 0) {
        (void)posix_spawn_file_actions_destroy(&file_actions);
        return (struct lane_result){255, "launch_failed", false};
    }

#if defined(__APPLE__)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    int cwd_result = posix_spawn_file_actions_addfchdir_np(&file_actions, working_directory);
#pragma clang diagnostic pop
#else
    int cwd_result = posix_spawn_file_actions_addfchdir(&file_actions, working_directory);
#endif
    int action_result = cwd_result;
    if (action_result == 0) {
        action_result = posix_spawn_file_actions_addopen(
            &file_actions,
            STDIN_FILENO,
            "/dev/null",
            O_RDONLY,
            0
        );
    }
    if (action_result == 0) {
        action_result = posix_spawn_file_actions_addopen(
            &file_actions,
            STDOUT_FILENO,
            "/dev/null",
            O_WRONLY,
            0
        );
    }
    if (action_result == 0) {
        action_result = posix_spawn_file_actions_addopen(
            &file_actions,
            STDERR_FILENO,
            "/dev/null",
            O_WRONLY,
            0
        );
    }
#if defined(__APPLE__)
    short spawn_flags = POSIX_SPAWN_CLOEXEC_DEFAULT;
#else
    short spawn_flags = 0;
#endif
    if (action_result == 0) {
        action_result = posix_spawnattr_setflags(&attributes, spawn_flags);
    }

    pid_t process = -1;
    int spawn_result = action_result;
    if (spawn_result == 0) {
        // DUX-DESTRUCTIVE: allow=spike-ai-reviewed-child-spawn -- non-shipping harness spawns only its fixed child or fixed deprecated comparison wrapper
        spawn_result = posix_spawn(
            &process,
            executable,
            &file_actions,
            &attributes,
            arguments,
            environment
        );
    }
    (void)posix_spawn_file_actions_destroy(&file_actions);
    (void)posix_spawnattr_destroy(&attributes);
    if (spawn_result != 0) {
        if (use_deprecated_sandbox && spawn_result == ENOENT) {
            return (struct lane_result){SANDBOX_EXEC_UNAVAILABLE, "unavailable", false};
        }
        return (struct lane_result){255, "launch_failed", false};
    }

    int status = 0;
    while (waitpid(process, &status, 0) < 0) {
        if (errno != EINTR) {
            return (struct lane_result){255, "wait_failed", false};
        }
    }
    int exit_code = normalized_wait_status(status);
    if (exit_code == CHILD_READ_SUCCEEDED) {
        return (struct lane_result){exit_code, "ambient_read_succeeded", use_deprecated_sandbox};
    }
    if (exit_code == CHILD_READ_DENIED) {
        return (struct lane_result){exit_code, "read_denied", use_deprecated_sandbox};
    }
    if (use_deprecated_sandbox && exit_code == SANDBOX_EXEC_UNAVAILABLE) {
        return (struct lane_result){exit_code, "unavailable", false};
    }
    return (struct lane_result){exit_code, "probe_failed", false};
}

static bool same_identity(const struct stat *left, const struct stat *right) {
    return left->st_dev == right->st_dev && left->st_ino == right->st_ino &&
           (left->st_mode & S_IFMT) == (right->st_mode & S_IFMT);
}

static bool entry_matches(int directory, const char *name, const struct stat *expected) {
    struct stat observed;
    return fstatat(directory, name, &observed, AT_SYMLINK_NOFOLLOW) == 0 &&
           same_identity(&observed, expected);
}

struct fixture_state {
    int temporary_directory;
    int root_directory;
    int private_directory;
    int working_directory;
    const char *root_name;
    bool root_created;
    bool root_tracked;
    bool private_created;
    bool private_tracked;
    bool working_created;
    bool working_tracked;
    bool sentinel_created;
    bool sentinel_tracked;
    struct stat root_identity;
    struct stat private_identity;
    struct stat working_identity;
    struct stat sentinel_identity;
};

static bool cleanup_owned_root(struct fixture_state *fixture) {
    bool clean = true;
    if (fixture->sentinel_created) {
        if (!fixture->sentinel_tracked || fixture->private_directory < 0 ||
            !entry_matches(
                fixture->private_directory,
                "sentinel",
                &fixture->sentinel_identity
            )) {
            clean = false;
        } else {
            // DUX-DESTRUCTIVE: allow=spike-ai-sentinel-cleanup -- identity-checked unlinkat removes only the pinned disposable sentinel
            if (unlinkat(fixture->private_directory, "sentinel", 0) != 0) {
                clean = false;
            }
        }
    }
    if (fixture->private_created) {
        if (!fixture->private_tracked || fixture->root_directory < 0 ||
            !entry_matches(
                fixture->root_directory,
                "private",
                &fixture->private_identity
            )) {
            clean = false;
        } else {
            // DUX-DESTRUCTIVE: allow=spike-ai-private-dir-cleanup -- identity-checked unlinkat removes only the pinned empty private directory
            if (unlinkat(fixture->root_directory, "private", AT_REMOVEDIR) != 0) {
                clean = false;
            }
        }
    }
    if (fixture->working_created) {
        if (!fixture->working_tracked || fixture->root_directory < 0 ||
            !entry_matches(
                fixture->root_directory,
                "empty-workdir",
                &fixture->working_identity
            )) {
            clean = false;
        } else {
            // DUX-DESTRUCTIVE: allow=spike-ai-work-dir-cleanup -- identity-checked unlinkat removes only the pinned empty working directory
            if (unlinkat(fixture->root_directory, "empty-workdir", AT_REMOVEDIR) != 0) {
                clean = false;
            }
        }
    }
    if (fixture->root_created) {
        if (!fixture->root_tracked || fixture->temporary_directory < 0 ||
            fixture->root_name == NULL ||
            !entry_matches(
                fixture->temporary_directory,
                fixture->root_name,
                &fixture->root_identity
            )) {
            clean = false;
        } else {
            // DUX-DESTRUCTIVE: allow=spike-ai-root-cleanup -- parent-fd identity check confines removal to the unique pinned probe root
            if (unlinkat(
                    fixture->temporary_directory,
                    fixture->root_name,
                    AT_REMOVEDIR
                ) != 0) {
                clean = false;
            }
        }
    }
    if (fixture->private_directory >= 0) {
        (void)close(fixture->private_directory);
    }
    if (fixture->working_directory >= 0) {
        (void)close(fixture->working_directory);
    }
    if (fixture->root_directory >= 0) {
        (void)close(fixture->root_directory);
    }
    if (fixture->temporary_directory >= 0) {
        (void)close(fixture->temporary_directory);
    }
    return clean;
}

int main(int argument_count, char **arguments) {
    char child[PATH_MAX];
    if (argument_count != 1 || !reviewed_child_path(arguments[0], child, sizeof(child))) {
        (void)fprintf(stderr, "host and reviewed hostile-child must be adjacent absolute paths\n");
        return 64;
    }

    mode_t previous_mask = umask(077);
    struct fixture_state fixture = {
        .temporary_directory = -1,
        .root_directory = -1,
        .private_directory = -1,
        .working_directory = -1,
    };
    fixture.temporary_directory = open(
        "/private/tmp",
        O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
    );
    if (fixture.temporary_directory < 0) {
        (void)fprintf(stderr, "could not pin the disposable probe parent\n");
        return 65;
    }
    char root[] = "/private/tmp/dux-ai-subprocess-confinement.XXXXXX";
    if (mkdtemp(root) == NULL) {
        (void)fprintf(stderr, "could not create disposable probe root\n");
        (void)close(fixture.temporary_directory);
        return 65;
    }
    fixture.root_created = true;
    fixture.root_name = strrchr(root, '/');
    if (fixture.root_name != NULL) {
        fixture.root_name++;
        fixture.root_tracked = fstatat(
            fixture.temporary_directory,
            fixture.root_name,
            &fixture.root_identity,
            AT_SYMLINK_NOFOLLOW
        ) == 0;
    }
    fixture.root_directory = openat(
        fixture.temporary_directory,
        fixture.root_name == NULL ? "" : fixture.root_name,
        O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
    );
    struct stat opened_root;
    bool root_pinned = fixture.root_tracked && fixture.root_directory >= 0 &&
                       fstat(fixture.root_directory, &opened_root) == 0 &&
                       same_identity(&opened_root, &fixture.root_identity);
    if (!root_pinned) {
        (void)fprintf(stderr, "could not pin disposable probe root\n");
        (void)umask(previous_mask);
        (void)cleanup_owned_root(&fixture);
        return 65;
    }

    char private_directory[PATH_MAX] = {0};
    char working_directory[PATH_MAX] = {0};
    char sentinel[PATH_MAX] = {0};
    bool paths_valid =
        join_path(private_directory, sizeof(private_directory), root, "private") &&
        join_path(working_directory, sizeof(working_directory), root, "empty-workdir") &&
        join_path(sentinel, sizeof(sentinel), private_directory, "sentinel");
    if (!paths_valid) {
        (void)fprintf(stderr, "could not create disposable probe fixtures\n");
        (void)umask(previous_mask);
        (void)cleanup_owned_root(&fixture);
        return 66;
    }
    if (mkdirat(fixture.root_directory, "private", 0700) == 0) {
        fixture.private_created = true;
        fixture.private_tracked = fstatat(
            fixture.root_directory,
            "private",
            &fixture.private_identity,
            AT_SYMLINK_NOFOLLOW
        ) == 0;
    }
    if (fixture.private_tracked) {
        fixture.private_directory = openat(
            fixture.root_directory,
            "private",
            O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
        );
    }
    struct stat opened_private;
    bool private_pinned = fixture.private_directory >= 0 &&
                          fstat(fixture.private_directory, &opened_private) == 0 &&
                          same_identity(&opened_private, &fixture.private_identity);
    if (private_pinned && mkdirat(fixture.root_directory, "empty-workdir", 0700) == 0) {
        fixture.working_created = true;
        fixture.working_tracked = fstatat(
            fixture.root_directory,
            "empty-workdir",
            &fixture.working_identity,
            AT_SYMLINK_NOFOLLOW
        ) == 0;
    }
    if (fixture.working_tracked) {
        fixture.working_directory = openat(
            fixture.root_directory,
            "empty-workdir",
            O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
        );
    }
    struct stat opened_working;
    bool working_pinned = fixture.working_directory >= 0 &&
                          fstat(fixture.working_directory, &opened_working) == 0 &&
                          same_identity(&opened_working, &fixture.working_identity);
    if (!private_pinned || !working_pinned) {
        (void)fprintf(stderr, "could not pin disposable probe fixtures\n");
        (void)umask(previous_mask);
        (void)cleanup_owned_root(&fixture);
        return 66;
    }

    static const char canary[] = "DUX-CONFINEMENT-CANARY-V1";
    int sentinel_descriptor = openat(
        fixture.private_directory,
        "sentinel",
        O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
        0600
    );
    fixture.sentinel_created = sentinel_descriptor >= 0;
    bool sentinel_ready = sentinel_descriptor >= 0 &&
                          write_all(sentinel_descriptor, canary, sizeof(canary) - 1) &&
                          fsync(sentinel_descriptor) == 0;
    fixture.sentinel_tracked = sentinel_descriptor >= 0 &&
                               fstat(sentinel_descriptor, &fixture.sentinel_identity) == 0;
    sentinel_ready = sentinel_ready && fixture.sentinel_tracked;
    if (sentinel_descriptor >= 0) {
        (void)close(sentinel_descriptor);
    }
    (void)umask(previous_mask);
    if (!sentinel_ready) {
        (void)fprintf(stderr, "could not write disposable probe sentinel\n");
        (void)cleanup_owned_root(&fixture);
        return 67;
    }

    struct lane_result direct = run_child(child, sentinel, fixture.working_directory, false);
    struct lane_result deprecated = run_child(child, sentinel, fixture.working_directory, true);
    bool cleanup_complete = cleanup_owned_root(&fixture);

    char product_version[64] = {0};
    char build_version[64] = {0};
    char architecture[64] = {0};
    platform_facts(
        product_version,
        sizeof(product_version),
        build_version,
        sizeof(build_version),
        architecture,
        sizeof(architecture)
    );

#if defined(__APPLE__)
    const char *product_name = "macOS";
#else
    const char *product_name = "other";
#endif
    const bool no_go = direct.child_exit_code == CHILD_READ_SUCCEEDED;
    (void)printf(
        "{\"schema_version\":1,\"probe_revision\":1,"
        "\"platform\":{\"product_name\":\"%s\",\"product_version\":\"%s\","
        "\"build_version\":\"%s\",\"architecture\":\"%s\"},"
        "\"controls\":{\"sentinel_mode\":\"0600\",\"clean_environment\":true,"
        "\"empty_working_directory\":true,\"nonstandard_descriptors_closed\":true,"
        "\"known_absolute_path_disclosed\":true,\"canary_content_emitted\":false,"
        "\"cleanup_complete\":%s},"
        "\"direct\":{\"result\":\"%s\",\"child_exit_code\":%d,\"confined\":%s},"
        "\"deprecated_sandbox_exec\":{\"available\":%s,\"applied\":%s,"
        "\"result\":\"%s\",\"child_exit_code\":%d,\"production_eligible\":false},"
        "\"decision\":{\"direct_local_adapter\":\"%s\"}}\n",
        product_name,
        product_version,
        build_version,
        architecture,
        cleanup_complete ? "true" : "false",
        direct.outcome,
        direct.child_exit_code,
        no_go ? "false" : "null",
        deprecated.child_exit_code != SANDBOX_EXEC_UNAVAILABLE ? "true" : "false",
        deprecated.applied ? "true" : "false",
        deprecated.outcome,
        deprecated.child_exit_code,
        no_go ? "no_go" : "inconclusive"
    );

    return no_go && cleanup_complete ? 0 : 1;
}
