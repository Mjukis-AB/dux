# macOS release and Sparkle key operations

Status: implemented non-publishing release qualification scaffold; production
credential configuration, custody backup, recovery drill, retained notarized
release, update feed, and public publication remain operational gates.

This runbook is the operational companion to
[ADR 0002](adr/0002-direct-developer-id-distribution.md). It does not authorize
a release or a private-key export. It defines how authorized operators prepare,
execute, verify, and recover the DUX direct-distribution lane without mixing it
with the standalone CLI release.

## Frozen public identity

Every production release must match
`dux-macos/Config/ProductionIdentity.json` exactly:

| Field | Required value |
| --- | --- |
| App bundle identifier | `se.mjukis.dux` |
| Apple Team ID | `SMQ3E8Y57T` |
| Developer ID identity | `Developer ID Application: MJUKIS AB (SMQ3E8Y57T)` |
| Sparkle Keychain account | `se.mjukis.dux` |
| Sparkle Ed25519 public key | `UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo=` |

The Sparkle key is DUX-specific. It is not the Claudex key and must never be
shared with Claudex or another product. The private half currently exists only
in the release owner's login Keychain. No private-key export, backup, CI import,
or recovery drill was performed while creating this runbook.

Debug uses `se.mjukis.dux.spike`. A Debug build is never evidence for TCC,
Launch at Login, Developer ID, notarization, stable installation, or update
identity.

## Authority separation

The following lanes are separate by design:

1. `.github/workflows/release.yml` publishes the cross-platform standalone CLI.
   It has no Apple, notarization, Xcode, or Sparkle dependency.
2. `.github/workflows/release-macos-app.yml` manually builds, signs, notarizes,
   staples, and qualifies a macOS DMG across two ephemeral macOS runners. The
   credential-free runner transfers only a checksummed unsigned envelope as a
   one-day GitHub Actions artifact. Because this repository is public, that
   intermediate is repository-readable and is not private custody. The fresh
   protected runner uploads no signed DMG or notarization log and publishes no
   GitHub Release, appcast, Homebrew artifact, or crate. A successful run is
   qualification evidence, not a retrievable release enclosure.
3. A future protected update-publication lane will create and publish the
   signed Sparkle appcast only after the DMG lane and update qualification pass.
   It does not exist yet.

Failure or unavailable credentials in one lane must not grant authority to,
skip checks in, or silently trigger another lane.

## Roles and approvals

Assign named humans before storing production secrets:

- **Release Owner** selects the reviewed tag/build number, obtains release
  approval, starts the protected workflow, and signs the final attestation.
- **Reviewer** verifies source/tag provenance, changelog, test evidence, and
  artifact manifest. The workflow initiator cannot be the sole reviewer.
- **Sparkle Custodian A** owns one offline backup and participates in restore
  drills and rotations.
- **Sparkle Custodian B** owns the second independently stored offline backup
  and witnesses export, recovery, and destruction ceremonies.
- **Incident Lead** can freeze publication, remove a feed item, revoke CI
  access, and coordinate Apple/Sparkle credential response. This role does not
  authorize a lower-version rollback.

One person may hold more than one role in a small team, but private-key export,
restore drills, rotation, and compromise response require two humans. Record
who approved the event, never secret material.

## Configure the protected GitHub environment

Repository administrators must create an environment named exactly
`macos-release-signing`. This is external GitHub state and cannot be proven by
repository tests. Before the first run:

1. Require at least one reviewer other than the workflow initiator and enable
   prevention of self-review.
2. Restrict deployments to protected stable tags matching `vX.Y.Z`; protect
   creation and movement of release tags through repository rules.
3. Disable administrator bypass where the repository plan supports it.
4. Keep environment secrets unavailable to pull requests, forks, ordinary CI,
   and the standalone CLI workflow.
5. Add only these macOS signing/notarization environment secrets:

   | Secret | Contents |
   | --- | --- |
   | `DUX_DEVELOPER_ID_P12_BASE64` | Base64 of the Developer ID Application PKCS#12 enclosure |
   | `DUX_DEVELOPER_ID_P12_PASSWORD` | PKCS#12 import password |
   | `DUX_NOTARY_API_KEY_P8_BASE64` | Base64 of the App Store Connect notarization API `.p8` key |
   | `DUX_NOTARY_KEY_ID` | Ten-character App Store Connect key ID |
   | `DUX_NOTARY_ISSUER_ID` | App Store Connect issuer UUID |

6. Grant the notarization API key only the access Apple requires for
   notarization. Record its owner, creation date, and review date outside the
   repository without copying the key.
7. Do not add a Sparkle private-key secret yet. After the offline backup and
   restore drill pass, the future appcast lane may use a protected secret named
   `DUX_SPARKLE_ED25519_PRIVATE_KEY_BASE64`. The DMG workflow must continue not
   to reference it.

The unprotected preparation job completes every dependency fetch, test,
generator, and unsigned Debug/Release build without access to release secrets.
Immediately before sealing, it repeats the clean-worktree, exact HEAD, and tag
checks. It archives the unsigned Release app, binds all three envelope members
to a deterministic SHA-256 job output, and transfers exactly that envelope with
one-day retention. A fresh protected runner receives both the digest through
the independent job-output channel and the envelope through Actions artifact
storage. It requires `download-artifact`'s service digest check to succeed,
recomputes the three-member digest, and only then independently verifies the
manifest, app layout, code inventory, metadata, and Sparkle policy before the
credential step. The verifier executes no build, test, dependency, or bundled
DUX code.

Only then does the workflow import raw Apple credentials into a
random-password ephemeral file Keychain. `umask 077` applies before any secret
file or Keychain creation; raw decoded files are removed by a step-local trap,
and import failure attempts to lock and delete the Keychain. The
credential-bearing interval invokes only the reviewed script's minimal
`--sign-prepared` phase and Apple/system tools: it runs no Cargo, Python,
XcodeGen, Xcode build/test, dependency, or bundled DUX executable. An `always`
step attempts to lock and delete the Keychain before post-signing verification;
hosted-runner teardown is the final containment if cancellation prevents that
step from completing. No signed DMG or notarization log is uploaded.

## What the manual workflow proves

The workflow may run only through `workflow_dispatch` from an exact stable tag.
It validates `vX.Y.Z`, the `X.Y.Z` input, a positive build number, the Cargo
workspace version, the tag's exact commit, ancestry from the repository default
branch, and the frozen public identity before requesting environment approval.
The preparation and protected signing jobs then use:

- the exact validated commit and a full-history, credential-free checkout;
- full-SHA-pinned Node 24 `checkout` 6.0.2, `upload-artifact` 7.0.1, and
  fail-on-digest-mismatch `download-artifact` 8.0.1 actions;
- Rust 1.96.0 with both macOS targets;
- XcodeGen 2.44.1 from its release archive with a pinned SHA-256;
- `/Applications/Xcode_16.4.app` and exact Apple build `16F6` on `macos-15`;
- the committed Swift package lockfile with automatic resolution disabled;
- the fixed three-phase `dux-macos/scripts/release-notarized-dmg.sh` entry point;
- Keychain profile `dux-notary` in the ephemeral release Keychain; and
- the exact production bundle, Team, Developer ID, and designated requirement.

The credential-free preparation phase runs repository policy, Rust, native,
architecture, deployment-target, generated-file, Sparkle-policy, and app-layout
gates and revalidates the clean tag immediately before sealing exactly three
prepared-envelope files. The fresh signing runner's public verification phase
requires both the artifact-service digest and the independently transferred
three-member envelope digest, then revalidates the archive hash and manifest,
extracts exactly one app, and executes no project code. The credential-bearing
phase revalidates the envelope again, runs no build or test code, then performs
explicit inside-out signing, notarization, staple, Gatekeeper, checksum, and
image-layout gates. It materializes output only through a same-filesystem rename to
`target/dux-macos-release/vX.Y.Z/` after every gate succeeds.

The protected job accepts exactly seven files:

1. `DUX-X.Y.Z.dmg`
2. `DUX-X.Y.Z.dmg.sha256`
3. `release-manifest.txt`
4. `DUX-X.Y.Z-app-notarization.json`
5. `DUX-X.Y.Z-app-notarization-log.json`
6. `DUX-X.Y.Z-dmg-notarization.json`
7. `DUX-X.Y.Z-dmg-notarization-log.json`

The signing job rechecks the exact output inventory, one-line checksum, complete
20-line manifest, and both accepted notarization response/log pairs after the
Keychain is deleted. It then allows the ephemeral runner to destroy the output.
The only `upload-artifact` call occurs in the unprivileged preparation job and
contains the unsigned three-file envelope, never the seven signed outputs.
GitHub Actions artifacts follow repository access and are not a private custody
mechanism for this public repository. A retained signed release requires the
authorized local operation below or a future separately reviewed encrypted
restricted-storage lane.

## Release procedure

### Before dispatch

The Release Owner and Reviewer must jointly confirm:

- the worktree commit is reviewed and present on the protected main line;
- every DUX Cargo package version is the intended stable `X.Y.Z`;
- `CHANGELOG.md` describes user-visible and security changes;
- `Cargo.lock`, the Sparkle package lock, generated Xcode project, generated
  Swift binding, and production identity record are committed and reviewed;
- current policy/Rust/native/universal build gates pass on the candidate;
- the prior released app and candidate are available for update qualification;
- the Developer ID certificate and notarization API key are unexpired and have
  not been reported compromised;
- no existing immutable output or public artifact already uses this version;
- the standalone CLI release decision is explicit and not inferred from this
  app release; and
- `SUFeedURL` is absent unless every update activation gate below is already
  complete.

Create the immutable tag only after approval. Tag creation/push is an external
publication action and is not performed by repository automation in this
runbook.

### Dispatch and review

1. Dispatch from the exact tag with GitHub CLI; the Actions UI selector is
   documented as a branch selector and is not the release procedure:

   ```bash
   gh workflow run release-macos-app.yml \
     --ref vX.Y.Z \
     -f version=X.Y.Z \
     -f build_number=N
   ```

2. Inspect the unprivileged validation and preparation jobs. Do not approve the
   protected environment if the tag, commit, policy suite, public identity,
   unsigned build, or envelope transfer fails.
3. The required Reviewer compares the requested tag/commit/build and prepared
   envelope artifact name with the
   release ticket and approves `macos-release-signing`.
4. Wait for the entire signing/notarization job. Do not retry a failed version
   by overwriting output. Diagnose the retained logs, correct source or
   credentials, and use a new build/version when immutable bytes may differ.

### Qualification review

After workflow success, two reviewers must compare the run's validated commit,
toolchain, test counts, accepted notarization statuses, and final manifest
checks with the release ticket. The workflow retains only the short-lived,
unsigned prepared envelope; it retains no downloadable signed release bytes.
To produce a release enclosure, two reviewers must supervise the local operation
below and then:

1. confirm the local immutable directory contains exactly the seven files
   above and no hidden/private-key
   material;
2. verify `shasum -a 256 -c DUX-X.Y.Z.dmg.sha256`;
3. compare every identity, version, build, commit, Xcode, Sparkle, CLI schema,
   architecture, deployment-target, and checksum line in
   `release-manifest.txt` with the release ticket;
4. inspect both notarization status records and full logs for `Accepted` and no
   error-severity issue;
5. mount the DMG on a disposable macOS account, verify the Applications link,
   install DUX into `/Applications`, and run Gatekeeper/signature/staple checks;
6. perform Apple Silicon and Intel fresh-install, TCC, Launch at Login,
   menu-bar, Explorer, optional CLI, and uninstall qualification; and
7. record only hashes, public identity, result, date, operators, and associated
   workflow qualification URL in the release attestation.

Public upload, GitHub Release creation, website changes, Homebrew Cask changes,
appcast publication, or user notification require separate explicit approval.

## Local release operation

Use a credential-free account or host for preparation. From an exact clean
`vX.Y.Z` tag, create the sealed unsigned envelope before unlocking or importing
any Apple release credential:

```bash
DUX_VERSION=X.Y.Z \
DUX_BUILD_NUMBER=1 \
DUX_BUNDLE_IDENTIFIER=se.mjukis.dux \
DUX_TEAM_ID=SMQ3E8Y57T \
./dux-macos/scripts/release-notarized-dmg.sh \
  --prepare /canonical/restricted/path/dux-X.Y.Z-prepared
```

Transfer the exact three-file envelope to the authorized signing account or
host without modifying it. Record the full tag commit, store notarization
credentials in a dedicated Keychain, and only then run the signing phase:

```bash
commit=FULL_40_CHARACTER_TAG_COMMIT
xcrun notarytool store-credentials dux-notary

DUX_VERSION=X.Y.Z \
DUX_BUILD_NUMBER=1 \
DUX_BUNDLE_IDENTIFIER=se.mjukis.dux \
DUX_TEAM_ID=SMQ3E8Y57T \
DUX_SIGNING_IDENTITY='Developer ID Application: MJUKIS AB (SMQ3E8Y57T)' \
DUX_NOTARYTOOL_PROFILE=dux-notary \
DUX_RELEASE_COMMIT="$commit" \
./dux-macos/scripts/release-notarized-dmg.sh \
  --sign-prepared /canonical/restricted/path/dux-X.Y.Z-prepared
```

CI additionally sets `DUX_NOTARYTOOL_KEYCHAIN` to its dedicated file Keychain.
The variable is a Keychain path, never a `.p8` path. The script accepts no
password, Apple ID, or raw API private-key argument.

## Sparkle private-key custody ceremony

Do not perform this ceremony until the Release Owner, both Custodians, and an
approved encrypted offline destination are present. Never export to the
repository, home directory, Downloads, `/tmp`, clipboard, shell variable,
cloud-synced folder, CI log, workflow artifact, appcast host, or password
manager note.

Preparation:

1. Obtain two removable devices dedicated to DUX release recovery. Erase each
   as encrypted APFS with independent high-entropy passphrases held by different
   Custodians. Disable indexing and automatic cloud backup for the ceremony
   host and disconnect unnecessary networks.
2. Download only the official
   `Sparkle-for-Swift-Package-Manager.zip` for Sparkle 2.9.5. The reviewed
   package revision is `79bc9e872948e47877e76f194cb0c8e0412b0b90`, whose
   `Package.swift` pins SHA-256
   `34b9b2071f3de0012eca3faa3a9290bb94e62131e9a74f6dc91514a000097a6c`.
   Verify that exact ZIP checksum before extraction and invoke
   `Sparkle/bin/generate_keys` from the verified extraction, never an arbitrary
   `PATH` binary that merely prints version 2.9.5.
3. Read the Keychain public half without export:

   ```bash
   /verified/Sparkle/bin/generate_keys --account se.mjukis.dux -p
   ```

   It must equal the frozen public key above byte-for-byte.
4. Resolve the mounted volume with `pwd -P`, reject a symlinked mount, create a
   dedicated mode-`0700` directory on it under `umask 077`, and verify the
   directory's canonical parent remains below that mount. Choose a new fixed
   filename such as `dux-sparkle-ed25519-private-v1.txt` inside it.

With both Custodians watching, export directly to the first encrypted volume:

```bash
set -euo pipefail
set +x
umask 077
mount=/Volumes/DUX-CUSTODY-A
expected_volume_uuid='PASTE-RECORDED-VOLUME-UUID'
[[ "$expected_volume_uuid" =~ ^[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}$ ]]
test ! -L "$mount"
mount_real="$(cd "$mount" && pwd -P)"
test "$mount_real" = "$mount"
volume_info="$(diskutil info -plist "$mount_real")"
plist_field() { plutil -extract "$1" raw -o - - <<<"$volume_info"; }
test "$(plist_field VolumeUUID)" = "$expected_volume_uuid"
test "$(plist_field MountPoint)" = "$mount_real"
test "$(plist_field FilesystemType)" = apfs
test "$(plist_field Encryption)" = true
test "$(plist_field FileVault)" = true
test "$(plist_field Locked)" = false
test "$(plist_field Internal)" = false
test "$(plist_field RemovableMediaOrExternalDevice)" = true
test "$(plist_field WritableVolume)" = true
custody_dir="$mount_real/DUX-Sparkle-Custody"
mkdir "$custody_dir"
chmod 700 "$custody_dir"
test ! -L "$custody_dir"
test "$(cd "$custody_dir" && pwd -P)" = "$mount_real/DUX-Sparkle-Custody"
test "$(cd "$(dirname "$custody_dir")" && pwd -P)" = "$mount_real"
test "$(stat -f %Lp "$custody_dir")" = 700
destination="$custody_dir/dux-sparkle-ed25519-private-v1.txt"
test ! -e "$destination"
test ! -L "$destination"
/verified/Sparkle/bin/generate_keys --account se.mjukis.dux -x "$destination"
chmod 600 "$destination"
test ! -L "$destination"
test "$(stat -f %Lp "$destination")" = 600
test "$(stat -f %l "$destination")" = 1
test "$(stat -f %z "$destination")" = 44
test "$(cd "$(dirname "$destination")" && pwd -P)" = "$custody_dir"
```

Do not print or hash the plaintext private file. Lock/unmount the volume, then
repeat a separately authorized direct export to Custodian B's encrypted volume.
Never stage a common plaintext copy between them. Store the devices powered
off, geographically separate, and physically controlled by their Custodians.

The ceremony record contains only:

- date, Sparkle version, DUX public key, backup generation `v1`;
- the exact APFS Volume UUID for each encrypted external custody device and
  identifiers that do not reveal passphrases;
- Custodian/Reviewer names and signatures;
- confirmation that no intermediate plaintext was created; and
- the scheduled first recovery drill date.

Only after a successful recovery drill may the same private bytes be placed in
the protected future CI secret. Encode/import the secret through the GitHub UI
or an approved secret-management channel that does not expose it to shell
history or logs. Never add that secret to the current DMG workflow.

## Recovery drill

Run this drill before the first appcast and at least every six months. Alternate
Custodian A and B media. A backup does not count as recoverable until this drill
passes.

1. Create a disposable macOS user on a machine that has never held the DUX
   Sparkle key. Disconnect unnecessary network access and disable screen/log
   capture.
2. Obtain `generate_keys` and `sign_update` only from the checksum-pinned
   Sparkle ZIP above. Attach and unlock one custody volume, verify its recorded
   Volume UUID and encrypted external APFS fields, then unmount and remount that
   same device read-only before opening the backup.
3. With shell tracing and logging disabled, silently prevalidate the backup as
   canonical base64 that decodes to exactly 32 bytes. The validator must print
   and hash nothing from the seed. One bounded implementation is:

   ```bash
   set -euo pipefail
   set +x
   mount=/Volumes/DUX-CUSTODY-A
   expected_volume_uuid='PASTE-RECORDED-VOLUME-UUID'
   [[ "$expected_volume_uuid" =~ ^[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}$ ]]
   test ! -L "$mount"
   mount_real="$(cd "$mount" && pwd -P)"
   test "$mount_real" = "$mount"
   volume_info="$(diskutil info -plist "$mount_real")"
   plist_field() { plutil -extract "$1" raw -o - - <<<"$volume_info"; }
   test "$(plist_field VolumeUUID)" = "$expected_volume_uuid"
   test "$(plist_field MountPoint)" = "$mount_real"
   test "$(plist_field FilesystemType)" = apfs
   test "$(plist_field Encryption)" = true
   test "$(plist_field FileVault)" = true
   test "$(plist_field Locked)" = false
   test "$(plist_field Internal)" = false
   test "$(plist_field RemovableMediaOrExternalDevice)" = true
   device_identifier="$(plist_field DeviceIdentifier)"
   [[ "$device_identifier" =~ ^disk[0-9]+s[0-9]+$ ]]
   diskutil unmount "$device_identifier"
   diskutil mount readOnly "$device_identifier"
   test ! -L "$mount"
   mount_real="$(cd "$mount" && pwd -P)"
   test "$mount_real" = "$mount"
   volume_info="$(diskutil info -plist "$mount_real")"
   test "$(plist_field VolumeUUID)" = "$expected_volume_uuid"
   test "$(plist_field MountPoint)" = "$mount_real"
   test "$(plist_field FilesystemType)" = apfs
   test "$(plist_field Encryption)" = true
   test "$(plist_field FileVault)" = true
   test "$(plist_field Locked)" = false
   test "$(plist_field Internal)" = false
   test "$(plist_field RemovableMediaOrExternalDevice)" = true
   test "$(plist_field WritableVolume)" = false
   custody_dir="$mount_real/DUX-Sparkle-Custody"
   test -d "$custody_dir"
   test ! -L "$custody_dir"
   test "$(cd "$custody_dir" && pwd -P)" = "$mount_real/DUX-Sparkle-Custody"
   seed_path="$custody_dir/dux-sparkle-ed25519-private-v1.txt"
   test -f "$seed_path"
   test ! -L "$seed_path"
   test "$(stat -f %Lp "$seed_path")" = 600
   test "$(stat -f %l "$seed_path")" = 1
   test "$(stat -f %z "$seed_path")" = 44
   test "$(cd "$(dirname "$seed_path")" && pwd -P)" = "$custody_dir"
   SEED_PATH="$seed_path" python3 - <<'PY'
   import base64, binascii, os, stat
   path = os.environ["SEED_PATH"]
   descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
   try:
       status = os.fstat(descriptor)
       if not stat.S_ISREG(status.st_mode) or status.st_nlink != 1:
           raise SystemExit("seed must be a single-link regular file")
       if stat.S_IMODE(status.st_mode) != 0o600 or status.st_size != 44:
           raise SystemExit("seed must be an exact mode-0600 44-byte file")
       encoded = os.read(descriptor, 45)
       if len(encoded) != 44 or os.read(descriptor, 1):
           raise SystemExit("seed changed size while it was read")
   finally:
       os.close(descriptor)
   try:
       decoded = base64.b64decode(encoded, validate=True)
   except binascii.Error:
       raise SystemExit("seed is not canonical base64") from None
   if len(decoded) != 32 or base64.b64encode(decoded) != encoded:
       raise SystemExit("seed is not one canonical 32-byte Ed25519 seed")
   PY
   ```

4. Import directly from the encrypted volume into the disposable user's
   Keychain. Re-establish every path and read-only-volume assertion in the same
   fail-fast block; do not rely on variables from the validation snippet:

   ```bash
   set -euo pipefail
   set +x
   mount=/Volumes/DUX-CUSTODY-A
   expected_volume_uuid='PASTE-RECORDED-VOLUME-UUID'
   [[ "$expected_volume_uuid" =~ ^[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}$ ]]
   test ! -L "$mount"
   mount_real="$(cd "$mount" && pwd -P)"
   test "$mount_real" = "$mount"
   volume_info="$(diskutil info -plist "$mount_real")"
   plist_field() { plutil -extract "$1" raw -o - - <<<"$volume_info"; }
   test "$(plist_field VolumeUUID)" = "$expected_volume_uuid"
   test "$(plist_field MountPoint)" = "$mount_real"
   test "$(plist_field FilesystemType)" = apfs
   test "$(plist_field Encryption)" = true
   test "$(plist_field FileVault)" = true
   test "$(plist_field Locked)" = false
   test "$(plist_field Internal)" = false
   test "$(plist_field RemovableMediaOrExternalDevice)" = true
   test "$(plist_field WritableVolume)" = false
   custody_dir="$mount_real/DUX-Sparkle-Custody"
   test -d "$custody_dir"
   test ! -L "$custody_dir"
   test "$(cd "$custody_dir" && pwd -P)" = "$mount_real/DUX-Sparkle-Custody"
   seed_path="$custody_dir/dux-sparkle-ed25519-private-v1.txt"
   test -f "$seed_path"
   test ! -L "$seed_path"
   test "$(stat -f %Lp "$seed_path")" = 600
   test "$(stat -f %l "$seed_path")" = 1
   test "$(stat -f %z "$seed_path")" = 44
   test "$(cd "$(dirname "$seed_path")" && pwd -P)" = "$custody_dir"
   /verified/Sparkle/bin/generate_keys --account se.mjukis.dux -f "$seed_path"
   ```

   Run this only in an unlogged interactive ceremony. A malformed import may
   expose input in diagnostics, so never automate it in CI.
5. Print only the recovered public half and compare it byte-for-byte with
   `ProductionIdentity.json`:

   ```bash
   /verified/Sparkle/bin/generate_keys --account se.mjukis.dux -p
   ```

6. Create a non-release canary enclosure containing no user data and sign it
   with the checksum-pinned `sign_update --account se.mjukis.dux`. Extract only
   the public base64 signature from its output. Verify the canary with the
   repository's public-key-only CryptoKit tool, passing the frozen public key,
   signature, and canary path:

   ```bash
   PUBLIC_SIGNATURE=$(
     /verified/Sparkle/bin/sign_update \
       --account se.mjukis.dux \
       -p /path/to/non-release-canary
   )
   swift dux-macos/scripts/verify-sparkle-ed25519-signature.swift \
     --public-key-base64 'UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo=' \
     --signature-base64 "$PUBLIC_SIGNATURE" \
     --file /path/to/non-release-canary
   ```

   This verifier accepts no private key or Keychain account. Do not publish the
   canary, signature, or an appcast.
7. Delete the disposable macOS user and its Keychain using the operating
   system's account-removal flow. Do not manually search for or remove arbitrary
   files. Unmount and relock the custody volume.
8. Record only public key, canary digest, pass/fail, date, Sparkle version,
   backup generation, and witnesses. If any step fails, the backup is unusable;
   keep the updater dormant and repeat the custody ceremony after diagnosis.

## Update trust-model gate

Sparkle 2.9.5 deliberately supports key rotation. Its stock application-bundle
validator accepts an update when the old Sparkle archive signature **or** the
matching Apple code-signing identity validates, subject to additional integrity
and key-transition checks. With DUX's required signed feed, non-expiring feed-
signature failures, and pre-extraction verification, the Ed25519 key still
protects feed metadata. However, the stock archive path does not prove the
stronger DUX requirement that every accepted application update carries both
the old exact DUX Ed25519 signature and the old exact DUX Developer ID identity.

Therefore `SUFeedURL` must remain absent until one of these decisions is
reviewed, implemented, adversarially tested, and recorded in ADR 0002:

1. accept Sparkle's documented rotation trust model and update DUX's threat
   model/incident assumptions accordingly; or
2. add a maintainable, reviewed enforcement layer that proves both exact DUX
   identities without replacing Sparkle's downloader/installer safety.

Do not solve this by patching generated Sparkle binaries, disabling rotation
silently, weakening signed-feed settings, or adding an unreviewed custom
downloader.

## Update activation gates

The app currently embeds these fail-closed settings but no feed URL:

- exact bundle identifier `se.mjukis.dux`;
- exact DUX public key;
- `SURequireSignedFeed = true`;
- `SUVerifyUpdateBeforeExtraction = true`; and
- `SUSignedFeedFailureExpirationInterval = 0`.

The native adapter independently checks all five facts plus an HTTPS URL before
creating `SPUStandardUpdaterController`. Before adding `SUFeedURL`, complete:

1. the trust-model decision above;
2. two offline encrypted backups and a successful recovery drill;
3. protected appcast CI that receives private key bytes only through standard
   input (`generate_appcast --ed-key-file -`) and never a command argument;
4. a reviewed stable HTTPS origin with immutable enclosures and controlled feed
   writes;
5. exact feed, release-notes, enclosure-signature, length, URL, version,
   minimum-system, checksum, Developer ID, notarization, staple, and schema
   verification before publication;
6. prior-to-current signed update tests on Apple Silicon and Intel, including
   tampering, interruption, offline, read-only volume, App Translocation,
   withdrawn item, already-current, downgrade, TCC, Launch at Login, menu-bar
   relaunch, settings/history, and separately installed CLI preservation; and
7. an approved incident and rollback exercise.

Automatic update checks must continue to use Sparkle's explicit consent flow.
Automatic download/install remains user controlled. DUX never changes those
preferences silently.

## Incident response

### Suspected Sparkle private-key compromise

1. Freeze appcast and public release writes immediately. Preserve access logs
   and public artifacts; do not expose the suspected key during investigation.
2. Remove an untrusted feed item or point the feed to a previously signed safe
   state only after confirming that doing so does not create a downgrade claim.
3. Keep installed versions monotonic. Recovery is a higher-version corrective
   release, never a lower-version silent rollback.
4. Determine whether the Apple Developer ID identity is also compromised. Do
   not rotate both trust anchors in one untested transition.
5. Follow the reviewed Sparkle key-rotation path from a known-good signed app
   and verify old-to-transition-to-new updates on both architectures.
6. Update the production identity record/public key only through a reviewed ADR
   amendment and full update qualification. Notify users only with explicit
   publication authorization.

### Sparkle key loss without compromise

Keep the feed frozen. Recover from a tested custody copy. If neither copy can be
restored, do not generate a replacement and publish it opportunistically.
Use only the trust-model/rotation path explicitly accepted in ADR 0002; otherwise
ship a manually installed higher-version Developer ID release after user-facing
communication.

### Developer ID compromise or expiry

Freeze DMG and appcast publication, revoke/replace the Apple credential through
Apple, and re-qualify the designated requirement, TCC, Launch at Login, and
Sparkle transition. A new certificate with the same Team ID is not assumed to
preserve every frozen property without testing.

### Bad but uncompromised release

Withdraw the feed item if safe, preserve evidence, fix the source, and issue a
higher version. Do not overwrite the old enclosure, checksum, tag, or retained
release record and do not make DUX claim that already installed bytes
disappeared.

## Periodic audit

Quarterly, the Release Owner verifies:

- environment reviewers, deployment rules, and all secret owners;
- Apple certificate/API-key expiration and least privilege;
- custody media ownership and next six-month restore drill;
- the dedicated DUX public key still matches Keychain, plist, adapter, identity
  record, release script, and runbook;
- the pinned Sparkle release remains supported and has no superseding security
  release;
- full action SHA pins, Xcode/XcodeGen/Rust pins, and runner availability; and
- the standalone CLI lane still contains no app-release credential dependency.

Any drift keeps the affected lane closed until reviewed and corrected.
