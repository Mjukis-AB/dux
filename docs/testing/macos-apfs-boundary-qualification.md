# macOS APFS boundary qualification

This repository-owned qualification proves that DUX's current-account home and
filesystem-boundary witnesses fail closed across macOS APFS namespace changes.
It is deliberately nondestructive with respect to user data: it creates one
small disposable disk image in a randomly named directory beneath the current
OS-account home, mounts that image over an empty fixture directory, runs one
exact ignored Rust test, detaches the image, and removes only that fixture.

The qualification does not invoke a cleanup planner, journal, executor, FFI, or Swift.
It creates no cleanup plan, approval, receipt, history row, application process,
or filesystem effect against a cleanup candidate. No user-selected path,
environment-provided home, wildcard test filter, `sudo`, or force-detach option
is accepted.

## What the test proves

The exact test first observes the empty mount point as an ordinary same-mount
descendant of the account home. It captures and revalidates a
`TrustedHomeMountWitness`, establishing the positive control. Before the disk
image is attached it also proves that:

- a symlink spelling of that root is rejected during no-follow root capture;
- `/System/Volumes/Data/Users/<account>` identifies the same live home object as
  `/Users/<account>`, but its alternate firmlink-facing spelling cannot acquire
  the current-account home witness.

The test then writes a readiness marker outside the future mount point and
waits for at most 30 seconds. The harness mounts a newly created APFS image on
the exact empty directory and releases the test. The test requires all of the
following:

- the original same-mount witness revalidation returns `Changed`;
- the live root has a different filesystem identity from the reviewed root;
- a fresh `TrustedHomeMountWitness` for the nested mount returns
  `DifferentMount`;
- descriptor-relative snapshot capture from the account home returns
  `CrossVolume` for the mounted target.

Every production permanent-safe review requires the same canonical scan-root,
home/mount, and target-snapshot evidence before it can mint rule, plan, journal,
or effect authority. Failure at these earlier witnesses therefore proves that a
volume-crossing target cannot enter that authority graph; the qualification
does not need or expose a test-only executor.

## Run it

Requirements:

- a supported macOS host (the product minimum is macOS 14);
- Xcode command-line tools and the pinned Rust toolchain;
- permission to create, mount, and detach a local disposable APFS disk image;
- an account home on a filesystem that permits the initial same-mount witness.

From a clean checkout, run:

```sh
scripts/qualify-macos-apfs-boundaries.sh
```

The harness accepts no arguments. A passing run ends with these path-free
records:

```text
APFS boundary qualification passed.
same_mount_descendant=accepted
symlink_alias=refused
system_data_home_spelling=refused
retained_witness_after_nested_mount=changed
nested_mount_home_witness=refused
home_relative_cross_volume_snapshot=refused
```

A skipped or unavailable fixture is not passing evidence. Missing commands,
an absent macOS Data-volume spelling, a timeout, an early test exit, attach
failure, unexpected accepted boundary, or failed detach makes the harness fail.
The ordinary Rust suite keeps the test ignored because it cannot honestly
construct a real nested mount inside a hermetic unit-test runner.

## Cleanup and failure handling

The disk image and mount point exist only below a randomly generated directory
named `.dux-apfs-boundary-qualification.*` in the OS-account home. The exit trap
terminates only its own still-running Cargo test supervisor, detaches only its
exact mount point, and removes only its exact fixture root. It never uses force
detach.

If ordinary detach fails, the harness reports an error and deliberately
preserves the complete fixture path instead of removing through a live mount.
Inspect it with `hdiutil info`, detach that exact mount manually, and then remove
the printed fixture only after confirming it is no longer mounted. Such a run
does not count as qualification evidence.

For release evidence, record the exact DUX commit, host architecture,
`sw_vers` product/build values, Rust version, and the complete path-free output.
Run the same committed harness on each supported macOS boundary used by the
signed-app qualification matrix; one local pass does not substitute for the
separate signed Intel and Apple Silicon destructive-app protocol.
