# Signed-app destructive qualification

Status: repository lane implemented; credentialed device evidence not yet run.

This protocol qualifies the native, signed application boundary for real
Explorer Trash and the one approved Rust-target permanent-safe rule. It never
targets existing user data and does not authorize public Release cleanup.

## Fixed boundary

`CleanupQualification` is a Release-optimized, non-shipping configuration. It
uses the frozen production bundle ID and Team so signed installation,
Gatekeeper, Launch Services, and TCC behavior are real. It also carries:

- both `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` and
  `DUX_CLEANUP_QUALIFICATION`;
- display name **DUX Cleanup Qualification**;
- Info.plist protocol version `1`; and
- the exact clean source commit in signed bundle metadata.

The menu popover and Explorer must show **Signed cleanup qualification** and
**Disposable test data only**. Ordinary Release has neither compilation
condition, carries protocol `0`/source `none`, and is rejected by release
packaging if qualification metadata appears.

The lane adds no test-only effect API. Every action must traverse the normal
scan/review/confirmation/consent/FFI/journal/executor graph. AI stays disabled
and cannot supply discovery, explanation, grouping, approval, or execution.

## Required matrix and people

Run the exact same universal app bytes on both:

| Host | Required operating system |
| --- | --- |
| Intel Mac | macOS 14 or a later supported Intel release |
| Apple Silicon Mac | Newest macOS supported by DUX at qualification time |

Use a fresh disposable local account on each host. The account must contain no
normal DUX data, no installed public DUX, no Full Disk Access grant, no login
item, and no personal files. The Release Owner performs the run; a second
reviewer witnesses identity, fixture scope, confirmations, and results. Signing
and notarization credentials remain in a dedicated Keychain and are never
placed in the repository, `/tmp`, workflow artifacts, logs, or the fixture.

The same person may not silently convert this protocol into publication.
Installing the qualification app, using Apple credentials, performing real
Trash/permanent-safe effects, retaining an artifact, and deleting the
disposable account each require the operator's explicit external approval.

## Build the exact unsigned app

Start from a clean reviewed commit present on the protected main line. Use a
new canonical DerivedData directory outside the repository:

```sh
commit=FULL_40_CHARACTER_COMMIT
derived=/canonical/private/path/DUX-Cleanup-Qualification-DerivedData

DUX_QUALIFICATION_VERSION=X.Y.Z \
DUX_QUALIFICATION_BUILD_NUMBER=N \
DUX_QUALIFICATION_COMMIT="$commit" \
./dux-macos/scripts/build-cleanup-qualification-app.sh "$derived"
```

The builder regenerates the universal Rust/Swift boundary and bundled CLI with
its Rust target beneath the requested DerivedData directory, regenerates the
Xcode project, refuses tracked or untracked source drift, validates the CLI
metadata against the unsigned CLI bytes, and produces:

```text
<DerivedData>/Build/Products/CleanupQualification/DUX.app
```

It does not sign, notarize, install, launch, or clean up the app. Do not use a
caller-supplied build-setting override against Debug or Release as a substitute.

## Sign and notarize without publication

Use the exact Developer ID identity in `Config/ProductionIdentity.json`, the
reviewed empty `Config/Release.entitlements`, and the inside-out order already
enforced by `release-notarized-dmg.sh`:

1. verify the app contains exactly the main executable, bundled CLI, Sparkle,
   Autoupdate, Updater, Downloader, and Installer Mach-O files;
2. sign the bundled CLI with identifier `se.mjukis.dux.cli`, secure timestamp,
   and Hardened Runtime, then immediately perform the same atomic
   `dux-cli-bundled-metadata.json` SHA-256 rebind and complete schema/version/
   architecture validation as `sign_bundled_cli` in
   `release-notarized-dmg.sh`. Signing changes the CLI bytes; signing the outer
   app with the old manifest is a qualification failure and makes Settings
   reject CLI installation;
3. sign Installer, Autoupdate, Updater, Downloader (preserving its reviewed
   empty entitlement dictionary), then Sparkle.framework. Verify every nested
   code object has exactly the reviewed empty entitlement dictionary after
   signing; no additional entitlement is accepted;
4. sign the outer app with the reviewed empty entitlements, secure timestamp,
   Hardened Runtime, and no `--deep` signing;
5. verify strict all-architecture signatures and the frozen designated
   requirement;
6. submit a private ZIP of the app through the dedicated Keychain-backed
   `notarytool` profile, require `Accepted`, retain the response/log only in the
   approved restricted evidence location, and staple the app;
7. create one final private ZIP from the stapled app, compute its lowercase
   SHA-256, lock it against modification in the restricted evidence location,
   and transfer that exact archive to both hosts. Each reviewer verifies the
   archive SHA-256 before extraction; rebuilding, restapling, or rezipping per
   host is forbidden; and
8. extract that verified archive and copy its app to the previously absent
   exact path `/Applications/DUX Cleanup Qualification.app` on the disposable
   account.

Do not overwrite an existing application. Do not upload the signed app, ZIP,
notarization log, or qualification receipt to GitHub Actions or a public
release. Do not configure `SUFeedURL`.

Verify the installed bytes without launching them:

```sh
DUX_QUALIFICATION_VERSION=X.Y.Z \
DUX_QUALIFICATION_BUILD_NUMBER=N \
DUX_QUALIFICATION_COMMIT="$commit" \
DUX_QUALIFICATION_ARCHIVE_SHA256="$archive_sha256" \
./dux-macos/scripts/verify-signed-cleanup-qualification.sh \
  '/Applications/DUX Cleanup Qualification.app'
```

The verifier is read-only. It checks exact location and metadata, source
commit, universal main/CLI/Sparkle inventory, macOS 14 main/CLI minimum,
Developer ID/Team/runtime/timestamps, the signed CLI metadata-to-byte binding,
empty entitlements on all seven code objects, production designated
requirement, Sparkle's dormant signed-feed policy, notarization staple, and
Gatekeeper. It binds the independently verified final-archive SHA-256 into the
output. Record its nine path-free output lines verbatim.

## Disposable fixture

Create a new dedicated directory in the disposable account. The final leaf
names below are protocol constants; the enclosing account path is never copied
to the attestation.

```text
DUX-Qualification-Fixture/
├── trash-item.bin
└── rust-project/
    ├── Cargo.toml
    ├── Cargo.lock
    ├── src/main.rs
    └── target/
        ├── CACHEDIR.TAG
        └── debug/disposable-payload.bin
```

Requirements:

- generate `trash-item.bin` and `disposable-payload.bin` locally with known
  non-secret contents and record their SHA-256 and sizes;
- `Cargo.toml`, `Cargo.lock`, and `src/main.rs` are regular single-link files;
- `CACHEDIR.TAG` starts exactly with
  `Signature: 8a477f597d28d172789f06886806bc55`;
- the project, target, marker, and every target descendant are regular,
  symlink-free, single-link where applicable, and at least seven full days old;
- no Cargo or rustc process is running during review/execution; and
- the fixture contains no symlink, hard link, mount, cloud item, personal data,
  credential, repository checkout, or pre-existing build output.

The reviewer independently confirms the fixture is inside the disposable
account and records only fixed relative labels, hashes, sizes, and timestamps.
Do not use `/`, Home, a real project, an existing `target`, or another volume.

## Installed-app sequence

### 1. Launch and limited-access truth

Launch only the exact installed app. Confirm the qualification warning appears
in the menu popover and Explorer. Keep AI disabled and Full Disk Access
ungranted. Confirm the app remains useful, reports limited/unknown coverage
truthfully, and starts no cleanup merely because either surface opens.

### 2. Real Explorer Trash

Scan the fixture root, select `trash-item.bin` through Explorer, choose the
explicit Trash action, and verify the confirmation names Trash and says space
is not reclaimed until Trash is emptied. Confirm once.

Record that the original is absent and a terminal Trash history entry exists;
do not claim freed capacity. Use Finder **Put Back**, verify the restored file
has the original SHA-256/size, and confirm the history record remains an
immutable observation. Do not empty Trash. A scan root/directory boundary that
is ineligible for the action must remain unavailable or fail closed.

### 3. Dry check

In Settings → Project discovery roots, choose **Add project folder…** and add
only `rust-project`. Separately, under **Rust project discovery**, choose the
exact direct Cargo 1.96.0 executable, review its static identity, choose
**Review and enroll exact Cargo…**, and approve **Run once and enroll**. The
common rustup-proxy symlink is unsupported; failure to obtain a reviewed direct
Cargo executable fails this run.

Run a fresh targeted scan, open the exact Rust-target candidate, and prepare the
permanent-safe plan preview. Choose **Run dry check**.
Verify the result reports zero removed entries/bytes, no filesystem mutation,
and no capacity-reclamation claim. Re-hash the marker, manifests, source, and
payload; every value must match the baseline.

### 4. Permanent-safe cleanup

In Settings, explicitly enable permanent cleanup using the required phrase
`ENABLE PERMANENT CLEANUP`.
Return to the exact current candidate, prepare a fresh unexpired preview, and
review its current target, estimate, seven-day evidence, preserved target
directory/marker, warnings, and irreversibility. Accept the separate exact
destructive confirmation once.

Verify:

- the global task card survives navigation and reaches one terminal state;
- `target/debug/disposable-payload.bin` is absent;
- `target/` and `target/CACHEDIR.TAG` remain with the marker hash unchanged;
- `Cargo.toml`, `Cargo.lock`, and `src/main.rs` remain byte-identical;
- terminal history correlates to the task's exact path-free session;
- removed counts/bytes agree with the disposable baseline;
- verified capacity delta is shown only if the signed sampler produced one,
  otherwise it remains unknown; and
- the consumed preview cannot be started again. History offers read-only
  inspection, not retry.

Disable permanent cleanup before quitting. An outcome-unknown, unexpected
partial result, changed-since-plan refusal, missing history correlation, UI
crash, identity mismatch, or automatic retry fails the qualification. Preserve
only minimized path-free evidence and stop; never rerun the effect speculatively.

## Evidence record

One reviewed record per host contains only:

```text
protocol_version=1
source_commit=<40 lowercase hex>
archive_sha256=<64 lowercase hex>
version=<X.Y.Z>
build_number=<positive integer>
host_architecture=<x86_64|arm64>
macos_version=<product version and build>
team_id=SMQ3E8Y57T
cdhash=<40 hex>
executable_sha256=<64 lowercase hex>
gatekeeper=accepted-notarized-developer-id
installed_path=/Applications/DUX Cleanup Qualification.app
disposable_account=true
full_disk_access=false
ai_provider=disabled
qualification_warning=visible-menu-and-explorer
trash_original_sha256=<64 lowercase hex>
trash_put_back_sha256=<same 64 lowercase hex>
trash_history=terminal
dry_check=no-effect
permanent_result=completed
marker_preserved=true
project_files_preserved=true
history_correlation=exact
retry_surface=absent
reviewer=<named human>
observed_at=<UTC RFC 3339>
```

Do not record paths below the account, filenames beyond the fixed protocol
labels, database rows, snapshots, user names, Apple submissions, credentials,
or file contents. The Release Owner and Reviewer sign the result outside the
repository. Both host records must refer to the same commit, final private
archive SHA-256, and executable SHA-256. Archive equality is the evidence that
the complete signed, stapled bundle—not merely its main executable—was
identical on both hosts.

## Gate closure and cleanup

This gate passes only when both architecture records pass, the real Trash and
permanent-safe results are independently reviewed, and no unresolved failure
exists. Then update the roadmap and security matrix; do not yet remove the
Release compilation boundary.

After evidence review and separate approval, remove the qualification app and
destroy the disposable accounts/fixtures using the host's normal account
administration. This repository protocol never empties Trash, deletes the
fixture, changes TCC, or performs those external cleanup steps itself.
