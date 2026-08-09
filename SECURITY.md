# Security policy

DUX handles filesystem metadata and is being designed to perform narrowly
authorized cleanup. Security reports are welcome, especially when a problem
could expose private storage information, cross a cleanup authorization
boundary, or weaken the signed update path.

## Private reporting status

**Private vulnerability reporting is not active yet.** As of 2026-08-09, the
GitHub Private Vulnerability Reporting setting for this repository is disabled.
The repository therefore does not currently claim a project-operated private
reporting channel.

Do not put vulnerability details, private paths, logs containing personal
data, credentials, signing material, or a proof of concept in a public issue,
discussion, pull request, or commit. If a report cannot be made safely without
those details, retain it until the private channel is activated.

The intended channel is GitHub Private Vulnerability Reporting. Once the
repository Security tab shows **Report a vulnerability**, submit the report at:

<https://github.com/Mjukis-AB/dux/security/advisories/new>

Enabling the repository setting and verifying that this URL accepts a private
draft advisory are release gates. Merely adding this file does not activate the
channel. DUX does not publish an unverified security email address as a
substitute.

## Supported versions

| Surface | Security support |
| --- | --- |
| Latest published standalone CLI release | Receives security assessment and fixes when applicable. |
| Older standalone CLI releases | Unsupported; upgrade to the latest release unless a maintainer explicitly coordinates otherwise. |
| `main` and other unreleased builds | Reports are accepted, but these builds carry no stability or end-user support promise. |
| macOS app | No public production release is supported yet. Reports from development and qualification builds are still welcome. |

When the macOS app reaches public beta, this table must be updated in the same
change that publishes the release. A security fix may require upgrading to a
newer CLI or app version; compatibility with an affected old version is not a
promise to keep that version supported.

## What to report

Examples include:

- cleanup or eviction occurring without the exact current user review and
  confirmation;
- path, symlink, hard-link, mount, ownership, race, or protected-root checks
  that can be bypassed;
- a scan, history, diagnostic, notification, schedule, CLI, or AI value gaining
  cleanup authority;
- exposure of file paths, names, contents, snapshots, databases, credentials,
  or other private storage information;
- corruption or rollback that can turn stale state into current authority;
- signature, notarization, Sparkle feed, appcast, archive, CLI installer, or
  release-pipeline validation failures; and
- a denial-of-service flaw that can strand durable cleanup authority or make
  DUX repeat an uncertain filesystem effect.

Ordinary bugs that contain no sensitive security detail can use the public
issue tracker. If unsure, treat the report as private.

## Preparing a safe report

Include only what is needed to reproduce and assess the issue:

1. affected DUX version or commit, macOS version, and CPU architecture;
2. expected behavior, observed behavior, and security impact;
3. minimal, deterministic reproduction steps using disposable test data;
4. whether a filesystem effect occurred and whether its outcome is known; and
5. sanitized diagnostics or a minimized fixture, when useful.

Do not send real user files, a complete DUX database or snapshot, full home
directory listings, access tokens, Apple or Sparkle signing keys, notarization
credentials, or other secrets. Replace personal paths and names with stable
placeholders. If unsanitized evidence appears necessary, first coordinate its
scope and transfer inside the private advisory; do not attach it by default.

## Response expectations

After the private channel is active, maintainers target:

- acknowledgement within 3 business days;
- an initial impact and severity assessment within 7 business days; and
- a status update at least every 14 calendar days while the report remains
  active.

These are communication targets, not guaranteed remediation deadlines. The
time to a safe fix depends on impact, reproducibility, platform behavior, and
whether signing or update infrastructure is involved. Maintainers will say
when a target cannot be met rather than silently marking a report resolved.

## Coordinated disclosure

Reports remain private while the reporter and maintainers validate impact,
prepare a regression, fix the enforcing layer, and make a safe update or
mitigation available. The parties should agree on a disclosure date and on
what credit, if any, the reporter wants. A GitHub Security Advisory and CVE may
be used when warranted.

Do not publicly disclose exploit details before affected users have a
reasonable opportunity to update or apply guidance. If coordination stalls,
the reporter may request a revised timeline in the private advisory. DUX will
not ask a reporter to hide an unresolved issue indefinitely.

## Good-faith research

Use only systems and data you own or are explicitly authorized to test. Prefer
disposable fixtures and stop if testing encounters another person's data. Do
not exfiltrate data beyond the minimum proof, retain private data, disrupt
other users, perform social engineering, or test availability at scale.

Good-faith work within these limits will not be treated by the project as an
attack. This statement cannot authorize testing of third-party services,
Apple, GitHub, Sparkle infrastructure, or systems the maintainers do not own.

## Maintainer handling

For a credible cleanup or privacy report, maintainers must:

1. preserve only minimized, path-free evidence where possible;
2. assess whether the affected rule, mode, release, feed, or installer must be
   disabled;
3. identify the authority witness or enforcement layer that failed;
4. add a minimized regression before or with the fix;
5. fix the enforcing layer rather than only special-casing the reported path;
6. re-run the applicable cleanup and signed-release qualification; and
7. coordinate an advisory, update, key response, or recovery guidance when
   required.

The normative product boundary and incident steps live in
[SECURITY_DESIGN.md](SECURITY_DESIGN.md). Release operations are defined in
[docs/MACOS_RELEASE_OPERATIONS.md](docs/MACOS_RELEASE_OPERATIONS.md).

### Activating the channel

This is a maintainer-only external repository change. After explicit approval,
enable GitHub Private Vulnerability Reporting and verify the result:

```sh
gh api --method PUT repos/Mjukis-AB/dux/private-vulnerability-reporting
gh api repos/Mjukis-AB/dux/private-vulnerability-reporting
```

The verification response must contain `"enabled": true`. Then perform a
logged-out check that the Security policy is visible and a maintainer check
that **Report a vulnerability** opens a private draft advisory. Update this
file to say the channel is active, record the evidence in `ROADMAP.md`, and run
the repository policy tests before claiming the release gate complete.
