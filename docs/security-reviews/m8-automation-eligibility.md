# M8 automation eligibility security review

- Review date: 2026-08-09
- Scope: core automation eligibility policy revision 1, disabled-draft static
  policy preflight, UniFFI v63/overview v2, and native read-only presentation
- Result: approved as a non-authoritative eligibility and explanation boundary

## Decision

This checkpoint may evaluate bounded facts and explain why an inert draft is
blocked by shipped policy. It may not schedule, enable, approve, plan, or
execute cleanup. An `Eligible` core assessment is an observation only; no
cleanup API accepts it and it carries no authority-bearing value.

The production engine exposes only the immutable shipped-policy subset for a
stored disabled draft. It does not manufacture unavailable history or live
facts, and it does not expose the core `Eligible` decision over FFI. The native
phrase "Static checks passed; runtime checks not evaluated" is therefore the
strongest positive presentation admitted by this review.

## Complete core policy

Policy revision 1 always returns exactly these eight ordered gates:

1. the requested rule belongs to the draft's exact rule/category scope, is not
   excluded, and the exact current shipped rule is `SafeRegenerable`, uses
   `RemoveKnownRegenerableContents`, and is marked schedule-eligible;
2. the exact rule/scope has at least two successful manual runs;
3. the newest two matching attempts contain neither failure nor protected
   descendant;
4. the current candidate meets the stricter rule/draft minimum age;
5. the current candidate meets the stricter rule/draft minimum size;
6. fresh inactivity evidence exists when the exact shipped rule declares an
   activity guard;
7. complete current-policy scan/evaluation evidence and current validation are
   fresh and contain no protected descendant; and
8. runtime identity proves ordinary current-user execution, not `sudo` or an
   unsupported platform.

Current evidence and activity observations use the cleanup plan's existing
15-minute validity window. Future or stale clocks, timestamp inversion,
incomplete coverage, mismatched rules, outdated evaluation, unavailable
validation, protected descendants, detected activity, and privileged runtime
identity fail closed. The shipped rule—not caller input—determines whether an
activity guard is applicable.

Every gate is `Passed`, `NotApplicable`, `Blocked`, or `Unproven`. Any blocked
gate makes the overall result `Ineligible`; otherwise any unproven gate makes
it `Indeterminate`; only complete passing evidence can produce `Eligible`.
Absence is never interpreted as success.

## Static production projection

The overview recomputes shipped policy from the validated bundled catalog on
every read and binds one assessment to every exact ordered draft ID/revision.
Rule scope requires one exact current schedule-safe rule. Category scope counts
only schedule-safe rules in that category after exact current-revision
exclusions, and explains missing/stale rules, a category with no eligible rule,
or exclusion of all eligible rules.

UniFFI and Swift independently enforce:

- overview record version 2 and eligibility record/policy version 1;
- one assessment per draft in the same order, with exact ID/revision binding;
- at most 16 unique, canonically ordered closed-enum reasons;
- included rule count no greater than the bounded catalog total;
- nonempty reasons for `BlockedByStaticPolicy`; and
- a positive included count with no reasons for `AwaitingRuntimeEvidence`.

`AwaitingRuntimeEvidence` is deliberately not named eligible, enabled,
approved, or runnable. Settings is read-only and lists all remaining runtime
gates. No shipped rule is currently schedule-eligible, so current real drafts
remain statically blocked.

## Authority and history separation

Inputs and outputs are path-free and contain no candidate ID, scan handle,
plan, approval, journal lease, task, callback, filesystem witness, or effect.
The CLI, AI, app runtime, maintenance scheduler, planner, journal, and executor
do not consume the assessment.

The existing contract-v42 recurring-storage ranking groups historical sessions
by rule ID and derives a presentation threshold. It is not exact sealed
rule/scope attempt history, may omit failures through its success-oriented
aggregation, and is not accepted by the eligibility kernel. The next M8 slice
must query the newest attempts for the exact rule revision and sealed scope,
represent protected-descendant observations explicitly, and provide bounded
facts without converting history into authority.

## Verification evidence

The 2026-08-09 checkpoint passed all 22 focused core automation tests, all 141
runnable UniFFI tests with two unrelated cleanup-quiescence cases intentionally
ignored, all 869 native tests, all 130 repository policy tests, formatting,
locked workspace check, and warning-denied workspace Clippy. The
destructive-call audit found no unreviewed boundary in 405 repository source
files.

The deterministic non-registry core partition passed 1,232 tests with four
intentional host/performance helpers ignored. The serialized 410-test registry
partition passed 408 and exposed two pre-existing, moving macOS
FSEvents/live-revalidation timing cases in the production-shaped Rust-target
fixture; both exact cases passed unchanged in isolated runs, including one
that first reproduced and then passed on immediate retry. Earlier failures in
the same lane moved among other unchanged Rust-target cases while every exact
case passed. This is recorded as a test-harness isolation defect, not waived as
a green monolithic lane; the eligibility module and overview path do not
consume that fixture.

Debug and Release UniFFI generation produced byte-identical Swift at SHA-256
`75c2395cd030824d48be03af9fe41333d83b7f120537d7cabe69ffed782c7aa6`.
Clean Debug and Release apps are exact arm64/x86_64 universals, target macOS
14.0, retain `LSUIElement=true`, and embed Sparkle 2.9.5 plus the dedicated DUX
public key. Their bundled universal CLI is byte-identical at SHA-256
`c527bc7b10abbe4bce878b44de16e6d616a9e60385824b6907b5f03700de7f91`.
The generated Xcode project remained byte-identical before and after
regeneration at SHA-256
`cf4965b369c9861be84f6c5f2f0cef399f379682e57297adc3d3b702a260c83c`.

## Still excluded

This review does not approve schedule creation/editing UI, history-based draft
suggestions, persisted runtime fact adapters, a global kill switch, enabling,
pause controls, scheduler/wake behavior, notifications, fresh planning,
execution, failure pausing, or result history. Every actual future run must
still form a fresh plan and independently revalidate it immediately before any
effect.
