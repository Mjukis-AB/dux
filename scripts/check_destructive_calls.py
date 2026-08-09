#!/usr/bin/env python3
"""Fail when repository source adds an unreviewed destructive or process call."""

from __future__ import annotations

import argparse
import ast
import dataclasses
import pathlib
import re
import shlex
import stat
import subprocess
import sys
from collections.abc import Iterable


ANNOTATION = "DUX-DESTRUCTIVE"
ANNOTATION_RE = re.compile(
    rf"{ANNOTATION}:\s*allow=(?P<exception_id>[a-z][a-z0-9-]*)\s+--\s+"
    rf"(?P<reason>\S.*)\s*$"
)
MIN_REASON_LENGTH = 20

SCANNED_SUFFIXES = {
    ".bash",
    ".bat",
    ".cmd",
    ".command",
    ".c",
    ".cc",
    ".cpp",
    ".m",
    ".mm",
    ".pbxproj",
    ".py",
    ".ps1",
    ".rs",
    ".sh",
    ".swift",
    ".yaml",
    ".yml",
    ".zsh",
}

RUST_RULES = (
    (
        "rust-filesystem-effect",
        re.compile(
            r"\b(?:remove_file|remove_dir_all|remove_dir|rename)\s*\("
            r"|\b(?:std\s*::\s*)?fs\s*::\s*"
            r"(?:remove_file|remove_dir_all|remove_dir|rename)\b"
            r"|\buse\b[^;]{0,500}\b(?:remove_file|remove_dir_all|remove_dir|rename)\b",
            re.DOTALL,
        ),
    ),
    (
        "rust-platform-delete",
        re.compile(
            r"\b(?:unlink|unlinkat|rmdir|DeleteFile[AW]?|RemoveDirectory[AW]?|"
            r"SHFileOperation[AW]?)\s*\("
            r"|\b(?:libc|nix\s*::\s*libc|nix\s*::\s*unistd|rustix\s*::\s*fs)"
            r"\s*::\s*(?:remove|unlink|unlinkat|rmdir|renameat)\b"
            r"|\b(?:FileDispositionInfo(?:Ex)?|FILE_DISPOSITION_INFO|"
            r"SetFileInformationByHandle)\b"
            r"|\bnix\s*::\s*libc\s*::\s*SYS_renameat2\b"
            r"|(?<!fn )\brenameatx_np\s*\("
        ),
    ),
    (
        "rust-process-spawn",
        re.compile(
            r"\b(?:(?:std|tokio)\s*::\s*process\s*::\s*|async_process\s*::\s*)?"
            r"Command\s*::\s*(?:new|from)\b"
            r"|\b(?:libc|nix\s*::\s*libc)\s*::\s*"
            r"(?:system|popen|exec[a-z0-9_]*|posix_spawn[a-z0-9_]*)\b"
            r"|\b(?:CreateProcess|ShellExecute)[A-Z]*\s*\("
            r"|\buse\b[^;]{0,500}\b(?:std\s*::\s*)?process\s*::\s*Command\b"
        ),
    ),
    (
        "rust-truncation-effect",
        re.compile(
            r"\bFile\s*::\s*create\s*\(|\.\s*set_len\s*\("
            r"|\.\s*truncate\s*\(\s*true\s*\)"
        ),
    ),
)

SWIFT_RULES = (
    (
        "swift-filesystem-effect",
        re.compile(
            r"\b(?:removeItem|trashItem|evictUbiquitousItem|moveItem|replaceItem|recycle)\b"
            r"|\bperformFileOperation\s*\("
            r"|\b(?:Darwin|Glibc)\s*\.\s*(?:remove|unlink|rmdir)\s*\("
            r"|\bremoveItemAt(?:Path|URL)\b"
            r"|(?<!func )\b(?:unlinkat|renameat|renameatx_np)\s*\("
            r"|\.\s*write\s*\(\s*to\s*:"
        ),
    ),
    (
        "swift-process-spawn",
        re.compile(r"\b(?:Process|NSTask)\b"),
    ),
)

SHELL_RULES = (
    (
        "shell-remove",
        re.compile(
            r"(?<![A-Za-z0-9_.-])(?:/[A-Za-z0-9_./-]+/)?[\"']?"
            r"(?:r(?:(?:''|\"\")*)m|rmdir|unlink)[\"']?(?=\s)"
        ),
    ),
    (
        "shell-move",
        re.compile(r"(?<![A-Za-z0-9_.-])(?:/[A-Za-z0-9_./-]+/)?mv(?=\s)"),
    ),
    ("shell-find-delete", re.compile(r"\bfind\b[^\n]*\s-delete(?:\s|$)")),
    ("shell-git-clean", re.compile(r"\bgit\s+clean(?:\s|$)")),
    ("shell-truncate", re.compile(r"(?<![A-Za-z0-9_.-])truncate(?=\s)")),
    ("shell-eval", re.compile(r"(?:^|[\s;])eval(?:\s|$)|\bInvoke-Expression\b")),
    (
        "shell-command-string",
        re.compile(r"(?:^|[\s;])(?:/bin/)?(?:sh|bash|zsh)\s+-c(?:\s|$)|\bosascript\b"),
    ),
    (
        "powershell-remove",
        re.compile(
            r"\b(?:Remove-Item|Clear-Content)\b|\[System\.IO\.(?:File|Directory)\]::Delete"
        ),
    ),
    (
        "powershell-process-spawn",
        re.compile(
            r"\bStart-Process\b|\[System\.Diagnostics\.Process\]::Start|"
            r"\b(?:powershell|pwsh)\b[^\n]*\s-(?:Command|EncodedCommand)\b|"
            r"(?m:^\s*&\s+)"
        ),
    ),
    (
        "batch-filesystem-effect",
        re.compile(r"(?<![A-Za-z0-9_.-])(?:del|erase|rd|move)(?=\s)"),
    ),
    (
        "batch-process-spawn",
        re.compile(r"(?im)^\s*(?:start|call|cmd\s+/c|powershell|pwsh)(?=\s)"),
    ),
)

C_FAMILY_RULES = (
    (
        "c-filesystem-effect",
        re.compile(
            r"(?<![A-Za-z0-9_])(?:remove|unlink|unlinkat|rmdir|rename|renameat|"
            r"truncate|ftruncate|DeleteFile[AW]?|RemoveDirectory[AW]?|"
            r"MoveFile(?:Ex)?[AW]?)\s*\("
            r"|=\s*&?\s*(?:remove|unlink|unlinkat|rmdir|rename|renameat|"
            r"truncate|ftruncate|DeleteFile[AW]?|RemoveDirectory[AW]?|MoveFile(?:Ex)?[AW]?)\b"
            r"|(?m:^\s*#\s*define\s+[A-Za-z_][A-Za-z0-9_]*\s+"
            r"(?:remove|unlink|unlinkat|rmdir|rename|renameat|truncate|ftruncate|"
            r"DeleteFile[AW]?|RemoveDirectory[AW]?|MoveFile(?:Ex)?[AW]?)\b)"
        ),
    ),
    (
        "c-process-spawn",
        re.compile(
            r"(?<![A-Za-z0-9_])(?:system|popen|exec[a-z0-9_]*|posix_spawnp?|"
            r"CreateProcess[AW]?|ShellExecute[AW]?)\s*\("
            r"|=\s*&?\s*(?:system|popen|exec[a-z0-9_]*|posix_spawnp?|"
            r"CreateProcess[AW]?|ShellExecute[AW]?)\b"
            r"|(?m:^\s*#\s*define\s+[A-Za-z_][A-Za-z0-9_]*\s+"
            r"(?:system|popen|exec[a-z0-9_]*|posix_spawnp?|"
            r"CreateProcess[AW]?|ShellExecute[AW]?)\b)"
        ),
    ),
)

PYTHON_EFFECTS = {
    "os.remove",
    "os.removedirs",
    "os.rename",
    "os.renames",
    "os.replace",
    "os.rmdir",
    "os.system",
    "os.unlink",
    "pathlib.Path.rename",
    "pathlib.Path.replace",
    "pathlib.Path.rmdir",
    "pathlib.Path.unlink",
    "shutil.move",
    "shutil.rmtree",
    "subprocess.Popen",
    "subprocess.call",
    "subprocess.check_call",
    "subprocess.check_output",
    "subprocess.run",
}
PYTHON_EFFECT_ATTRIBUTES = {
    "Popen",
    "removedirs",
    "renames",
    "rmdir",
    "rmtree",
    "unlink",
}
PYTHON_PROCESS_ATTRIBUTES = {"call", "check_call", "check_output", "run", "system"}


@dataclasses.dataclass(frozen=True)
class Match:
    line: int
    rule: str
    snippet: str
    matched: str = ""


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    rule: str
    message: str

    def render(self) -> str:
        return f"{self.path}:{self.line}: [{self.rule}] {self.message}"


@dataclasses.dataclass(frozen=True)
class AllowAnnotation:
    line: int
    exception_id: str
    reason: str


@dataclasses.dataclass(frozen=True)
class ExceptionSpec:
    path: str
    rule: str
    context: str = "any"


EXCEPTIONS = {
    "finder-reveal": ExceptionSpec("dux-cli/src/app/state.rs", "rust-process-spawn", "open_in_finder"),
    "cargo-metadata-observer-spawn": ExceptionSpec(
        "dux-core/src/planner/rust_target_cargo/runner.rs",
        "rust-process-spawn",
        "run_cargo_portable",
    ),
    "cargo-suspended-observer-spawn": ExceptionSpec(
        "dux-core/src/planner/cargo_spawn_macos.rs", "rust-process-spawn", "spawn"
    ),
    "test-cargo-executable-ancestor-rename": ExceptionSpec(
        "dux-core/src/planner/cargo_spawn_macos.rs",
        "rust-filesystem-effect",
        "test:higher_executable_ancestor_rename_is_terminal",
    ),
    "test-cargo-workspace-ancestor-rename": ExceptionSpec(
        "dux-core/src/planner/cargo_workspace.rs",
        "rust-filesystem-effect",
        "test:higher_workspace_ancestor_rename_is_terminal",
    ),
    "test-cargo-closed-stdio-helper-spawn": ExceptionSpec(
        "dux-core/src/planner/cargo_spawn_macos.rs",
        "rust-process-spawn",
        "test:closed_standard_descriptors_cannot_alias_the_retained_cwd",
    ),
    "test-cargo-config-transient-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_config.rs",
        "rust-filesystem-effect",
        "test:create_then_remove_is_still_a_terminal_vnode_event",
    ),
    "test-target-namespace-ghost-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_target_namespace.rs",
        "rust-filesystem-effect",
        "test:target_create_remove_and_source_replacement_change_observation",
    ),
    "test-target-namespace-legacy-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_target_namespace.rs",
        "rust-filesystem-effect",
        "test:target_create_remove_and_source_replacement_change_observation",
    ),
    "test-target-namespace-source-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_target_namespace.rs",
        "rust-filesystem-effect",
        "test:target_create_remove_and_source_replacement_change_observation",
    ),
    "test-target-namespace-transient-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_target_namespace.rs",
        "rust-filesystem-effect",
        "test:directory_write_and_restore_is_terminal_for_armed_guard",
    ),
    "test-workspace-glob-member-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_workspace_glob.rs",
        "rust-filesystem-effect",
        "test:namespace_revalidation_detects_creation_and_removal",
    ),
    "test-workspace-glob-transient-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_workspace_glob.rs",
        "rust-filesystem-effect",
        "test:apfs_directory_note_write_is_terminal_even_when_name_is_removed",
    ),
    "test-manifest-probe-alias-replace": ExceptionSpec(
        "dux-core/src/planner/cargo_manifest_probes.rs",
        "rust-filesystem-effect",
        "test:aliases_and_non_regular_candidates_fail_closed",
    ),
    "test-manifest-probe-ancestor-rename": ExceptionSpec(
        "dux-core/src/planner/cargo_manifest_probes.rs",
        "rust-filesystem-effect",
        "test:ancestor_directory_rename_and_restore_is_terminal",
    ),
    "test-manifest-probe-replacement-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_manifest_probes.rs",
        "rust-filesystem-effect",
        "test:ancestor_directory_rename_and_restore_is_terminal",
    ),
    "test-manifest-probe-ancestor-restore": ExceptionSpec(
        "dux-core/src/planner/cargo_manifest_probes.rs",
        "rust-filesystem-effect",
        "test:ancestor_directory_rename_and_restore_is_terminal",
    ),
    "test-manifest-probe-unrelated-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_manifest_probes.rs",
        "rust-filesystem-effect",
        "test:absent_create_remove_is_terminal_but_unrelated_restoration_replays",
    ),
    "test-manifest-probe-absent-remove": ExceptionSpec(
        "dux-core/src/planner/cargo_manifest_probes.rs",
        "rust-filesystem-effect",
        "test:absent_create_remove_is_terminal_but_unrelated_restoration_replays",
    ),
    "test-cargo-cwd-replace": ExceptionSpec(
        "dux-core/src/planner/rust_target_cargo_tests.rs",
        "rust-filesystem-effect",
        "test:retained_working_directory_rejects_path_replacement",
    ),
    "test-cli-inspection-spawn": ExceptionSpec(
        "dux-cli/tests/inspection_cli.rs",
        "rust-process-spawn",
        "run_in_home",
    ),
    "test-persistence-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/persistence_tests.rs",
        "rust-process-spawn",
        "spawn_persistence_helper",
    ),
    "test-capacity-cross-process-helper-spawn": ExceptionSpec(
        "dux-core/src/engine/volume_status.rs",
        "rust-process-spawn",
        "test:newer_cross_process_durable_pressure_supersedes_local_ephemeral_baseline",
    ),
    "test-persistence-displace-shm": ExceptionSpec(
        "dux-core/src/persistence/persistence_tests.rs",
        "rust-filesystem-effect",
        "test:wal_is_recovered_after_crash_even_when_shared_memory_is_missing",
    ),
    "test-persistence-truncate-owned-database": ExceptionSpec(
        "dux-core/src/persistence/persistence_tests.rs",
        "rust-truncation-effect",
        "test:marker_owned_database_truncated_after_its_valid_header_is_corrupt",
    ),
    "permanent-safe-rust-target-descriptor-contents": ExceptionSpec(
        "dux-core/src/cleanup/permanent_safe.rs",
        "rust-platform-delete",
        "remove_contents_unix",
    ),
    "test-reviewed-trash-replaced-file-remove": ExceptionSpec(
        "dux-core/src/cleanup/executor.rs",
        "rust-filesystem-effect",
        "test:revalidation_rejects_replaced_object_identity",
    ),
    "test-reviewed-trash-missing-file-remove": ExceptionSpec(
        "dux-core/src/cleanup/executor.rs",
        "rust-filesystem-effect",
        "test:revalidation_rejects_missing_object_without_returning_a_path",
    ),
    "test-trash-admission-target-drift-remove": ExceptionSpec(
        "dux-core/src/persistence/cleanup_journal/tests.rs",
        "rust-filesystem-effect",
        "test:trash_admission_terminalizes_known_target_drift_before_releasing_the_claim",
    ),
    "test-cloud-probe-replace-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_cloud_probe_rejects_changed_target_before_callback",
    ),
    "test-cloud-probe-callback-replace-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_cloud_probe_rejects_target_replaced_during_callback",
    ),
    "test-snapshot-diff-removed-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_snapshot_diff_projects_lossless_union_and_separate_change_totals",
    ),
    "test-snapshot-diff-replaced-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_snapshot_diff_projects_lossless_union_and_separate_change_totals",
    ),
    "test-ffi-snapshot-diff-removed-rename": ExceptionSpec(
        "dux-ffi/src/lib.rs",
        "rust-filesystem-effect",
        "test:snapshot_diff_transport_is_parent_scoped_versioned_and_path_free",
    ),
    "test-descendant-policy-replacement-rename": ExceptionSpec(
        "dux-core/src/planner/descendant_policy.rs",
        "rust-filesystem-effect",
        "test:replacement_and_removal_fail_closed",
    ),
    "test-rule-scope-target-replacement-rename": ExceptionSpec(
        "dux-core/src/planner/rule_scope_grant.rs",
        "rust-filesystem-effect",
        "test:macos_grant_revalidates_exact_target_and_rejects_replacement",
    ),
    "test-exact-review-replacement-remove-temp": ExceptionSpec(
        "dux-core/src/planner/exact_path_review_tests.rs",
        "rust-filesystem-effect",
        "test:retained_review_boundary_rejects_scan_root_replacement",
    ),
    "test-exact-review-replacement-rename-away": ExceptionSpec(
        "dux-core/src/planner/exact_path_review_tests.rs",
        "rust-filesystem-effect",
        "test:retained_review_boundary_rejects_scan_root_replacement",
    ),
    "test-exact-review-replacement-remove-root": ExceptionSpec(
        "dux-core/src/planner/exact_path_review_tests.rs",
        "rust-filesystem-effect",
        "test:retained_review_boundary_rejects_scan_root_replacement",
    ),
    "test-exact-review-replacement-rename-back": ExceptionSpec(
        "dux-core/src/planner/exact_path_review_tests.rs",
        "rust-filesystem-effect",
        "test:retained_review_boundary_rejects_scan_root_replacement",
    ),
    "cache-write-failure-temp-remove": ExceptionSpec(
        "dux-core/src/cache/mod.rs", "rust-filesystem-effect", "save_cache"
    ),
    "cache-atomic-publish": ExceptionSpec("dux-core/src/cache/mod.rs", "rust-filesystem-effect", "save_cache"),
    "cache-publish-failure-temp-remove": ExceptionSpec(
        "dux-core/src/cache/mod.rs", "rust-filesystem-effect", "save_cache"
    ),
    "test-cache-first-temp-remove": ExceptionSpec(
        "dux-core/src/cache/mod.rs", "rust-filesystem-effect", "test:cache_temp_files_for_one_target_can_coexist"
    ),
    "test-cache-second-temp-remove": ExceptionSpec(
        "dux-core/src/cache/mod.rs", "rust-filesystem-effect", "test:cache_temp_files_for_one_target_can_coexist"
    ),
    "test-cache-reset-lock-helper-spawn": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-process-spawn",
        "test:reset_writer_admission_excludes_an_independent_process",
    ),
    "test-data-namespace-publication-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-process-spawn",
        "test:data_namespace_fence_excludes_subprocess_root_publication",
    ),
    "test-cache-reset-canonical-binding-rename": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-filesystem-effect",
        "test:reset_writer_admission_rejects_detached_canonical_cache_directory",
    ),
    "test-cache-recovery-already-detached-rename": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-filesystem-effect",
        "test:recovery_admission_accepts_an_exact_already_detached_store_without_repeating_effect",
    ),
    "test-cache-recovery-absence-stage-rename": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-filesystem-effect",
        "test:recovery_admission_rejects_cache_appearance_after_journaled_absence",
    ),
    "test-cache-recovery-both-names-rename": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-filesystem-effect",
        "test:recovery_admission_rejects_both_neither_and_wrong_identity_without_mutation",
    ),
    "test-cache-recovery-case-alias-rename": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-filesystem-effect",
        "test:recovery_admission_rejects_case_folded_canonical_and_stage_aliases",
    ),
    "test-cache-completed-case-alias-rename": ExceptionSpec(
        "dux-core/src/cache/managed_store/tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_rejects_case_folded_canonical_and_transaction_stage_aliases",
    ),
    "test-protected-replaced-root": ExceptionSpec(
        "dux-core/src/path_validation/protected/tests.rs",
        "rust-filesystem-effect",
        "test:replacement_scan_root_at_the_same_path_rejects_old_target_evidence",
    ),
    "test-engine-move-scan-root": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:scanner_failure_is_durable_and_publishes_no_snapshot",
    ),
    "test-subtree-traversal-root-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:subtree_scan_revalidates_identity_immediately_before_traversal",
    ),
    "test-subtree-publication-root-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:subtree_scan_revalidates_identity_immediately_before_publication",
    ),
    "test-targeted-scan-root-replacement-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:targeted_scan_join_requires_the_exact_root_identity",
    ),
    "test-live-target-replace-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_review_live_targets_are_purpose_bound_and_reject_stale_paths",
    ),
    "test-live-target-missing-remove": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_review_live_targets_are_purpose_bound_and_reject_stale_paths",
    ),
    "test-live-target-symlink-ancestor-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:explorer_review_live_target_rejects_a_replaced_symlink_ancestor",
    ),
    "test-candidate-recovery-missing-snapshot-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:candidate_evaluation_recovery_maintenance_maps_malformed_and_missing_snapshot_failures",
    ),
    "test-durable-rust-target-replace-rename": ExceptionSpec(
        "dux-core/src/planner/rust_target_source_tests.rs",
        "rust-filesystem-effect",
        "test:recreated_target_tree_is_rejected_even_when_the_paths_and_markers_match",
    ),
    "test-durable-rust-project-replace-rename": ExceptionSpec(
        "dux-core/src/planner/rust_target_source_tests.rs",
        "rust-filesystem-effect",
        "test:replaced_parent_is_rejected_even_when_original_terminal_objects_are_moved_back",
    ),
    "test-durable-rust-target-move-back": ExceptionSpec(
        "dux-core/src/planner/rust_target_source_tests.rs",
        "rust-filesystem-effect",
        "test:replaced_parent_is_rejected_even_when_original_terminal_objects_are_moved_back",
    ),
    "test-durable-rust-manifest-move-back": ExceptionSpec(
        "dux-core/src/planner/rust_target_source_tests.rs",
        "rust-filesystem-effect",
        "test:replaced_parent_is_rejected_even_when_original_terminal_objects_are_moved_back",
    ),
    "test-reset-old-store-nonmonotonic-control-remove": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-filesystem-effect",
        "test:old_database_store_retirement_rejects_non_monotonic_partial_controls",
    ),
    "storage-root-handle-publish": ExceptionSpec(
        "dux-core/src/persistence/storage/windows.rs",
        "rust-platform-delete",
        "rename_by_handle_no_replace",
    ),
    "storage-root-linux-publish": ExceptionSpec(
        "dux-core/src/persistence/storage.rs", "rust-platform-delete"
    ),
    "storage-root-macos-publish": ExceptionSpec(
        "dux-core/src/persistence/storage.rs", "rust-platform-delete"
    ),
    "app-data-reset-old-database-payload-unlink": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-platform-delete",
        "unlink_app_data_reset_old_database_payload_with_before_unlink",
    ),
    "app-data-reset-old-database-control-unlink": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-platform-delete",
        "unlink_app_data_reset_old_database_control_with_before_unlink",
    ),
    "app-data-reset-fresh-origin-unlink": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-platform-delete",
        "unlink_app_data_reset_fresh_origin_with_before_unlink",
    ),
    "app-data-reset-old-database-root-rmdir": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-platform-delete",
        "remove_app_data_reset_old_database_root_with_before_unlink",
    ),
    "snapshot-linux-no-replace-publish": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs", "rust-platform-delete"
    ),
    "snapshot-macos-no-replace-publish": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs", "rust-platform-delete"
    ),
    "snapshot-current-temp-unlink": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs", "rust-platform-delete"
    ),
    "snapshot-observed-final-unlink": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs", "rust-platform-delete"
    ),
    "app-data-reset-snapshot-control-unlink": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs",
        "rust-platform-delete",
        "remove_app_data_reset_snapshot_control_with_before_unlink",
    ),
    "app-data-reset-snapshot-directory-unlink": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs",
        "rust-platform-delete",
        "remove_app_data_reset_snapshot_directory_with_before_unlink",
    ),
    "snapshot-provisioning-stage-control-unlink": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs", "rust-platform-delete"
    ),
    "snapshot-provisioning-stage-rmdir": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/unix.rs", "rust-platform-delete"
    ),
    "snapshot-windows-current-temp-delete": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/windows.rs", "rust-platform-delete"
    ),
    "snapshot-windows-observed-final-delete": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/windows.rs", "rust-platform-delete"
    ),
    "snapshot-windows-provisioning-stage-delete": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/windows.rs",
        "rust-platform-delete",
        "set_posix_delete",
    ),
    "snapshot-windows-handle-publish": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/windows.rs", "rust-platform-delete"
    ),
    "managed-cache-entry-publish": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
        "publish_file_replace",
    ),
    "managed-cache-exact-clear": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
        "remove_retained_file",
    ),
    "managed-cache-exact-stage-remove": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
        "remove_retained_directory",
    ),
    "managed-cache-reset-stage-control-unlink": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
        "remove_app_data_reset_stage_control",
    ),
    "managed-cache-reset-stage-rmdir": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
        "remove_app_data_reset_retired_stage_directory",
    ),
    "managed-cache-linux-store-publish": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
    ),
    "managed-cache-macos-store-publish": ExceptionSpec(
        "dux-core/src/cache/managed_store/unix.rs",
        "rust-platform-delete",
    ),
    "app-data-reset-linux-coordinator-publish": ExceptionSpec(
        "dux-core/src/persistence/app_data_reset/storage.rs",
        "rust-platform-delete",
    ),
    "app-data-reset-macos-coordinator-publish": ExceptionSpec(
        "dux-core/src/persistence/app_data_reset/storage.rs",
        "rust-platform-delete",
    ),
    "app-data-reset-provisioning-stage-reconcile": ExceptionSpec(
        "dux-core/src/persistence/app_data_reset/storage.rs",
        "rust-platform-delete",
        "remove_provisioning_stage",
    ),
    "app-data-reset-journal-publish": ExceptionSpec(
        "dux-core/src/persistence/app_data_reset/storage.rs",
        "rust-platform-delete",
        "rename_stage_over_journal",
    ),
    "app-data-reset-exact-file-unlink": ExceptionSpec(
        "dux-core/src/persistence/app_data_reset/storage.rs",
        "rust-platform-delete",
        "remove_named_file",
    ),
    "test-snapshot-lock-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-process-spawn",
        "test:cross_process_writer_contention_is_bounded",
    ),
    "test-snapshot-reset-canonical-binding-rename": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:reset_revalidation_rejects_detached_canonical_snapshot_directory",
    ),
    "test-snapshot-temp-lock-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-process-spawn",
        "test:cross_process_temp_lock_proves_active_then_quiescent",
    ),
    "test-snapshot-durable-temp-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/snapshot/tests.rs",
        "rust-process-spawn",
        "test:cross_process_crash_preserves_row_and_transitions_temp_to_quiescent",
    ),
    "test-snapshot-inventory-fd-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-process-spawn",
        "test:inventory_closes_entry_handles_and_rejects_final_usage_change",
    ),
    "test-snapshot-umask-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-process-spawn",
        "test:restrictive_umask_still_provisions_exact_private_modes",
    ),
    "test-snapshot-macos-parent-acl-command": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-process-spawn",
        "test:macos_accepts_deny_only_parent_acl_and_rejects_final_object_acl",
    ),
    "test-snapshot-macos-final-acl-command": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-process-spawn",
        "test:macos_accepts_deny_only_parent_acl_and_rejects_final_object_acl",
    ),
    "test-snapshot-lock-command-import": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs", "rust-process-spawn"
    ),
    "test-snapshot-provisioning-stage-fixture-control-reset": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:wrong_marker_and_broad_stage_mode_fail_without_effect",
    ),
    "test-snapshot-provisioning-stage-fixture-directory-reset": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:wrong_marker_and_broad_stage_mode_fail_without_effect",
    ),
    "test-snapshot-provisioning-stage-file-reset": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:canonical_stage_file_and_symlink_fail_without_touching_targets",
    ),
    "test-snapshot-provisioning-stage-extra-reset": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:extra_or_linked_provisioning_stage_children_fail_without_effect",
    ),
    "test-reset-snapshot-marker-only-fixture": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_snapshot_store_rejects_non_monotonic_partial_shapes",
    ),
    "test-reset-snapshot-case-alias-intermediate-rename": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/app_data_reset.rs",
        "rust-filesystem-effect",
        "revalidate_structural_final_gate_until",
    ),
    "test-reset-snapshot-case-alias-final-rename": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/app_data_reset.rs",
        "rust-filesystem-effect",
        "revalidate_structural_final_gate_until",
    ),
    "test-reset-snapshot-case-alias-restore": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_snapshot_store_rejects_case_alias_at_the_final_gate",
    ),
    "test-snapshot-referenced-file-remove": ExceptionSpec(
        "dux-core/src/persistence/snapshot/tests.rs",
        "rust-filesystem-effect",
        "test:durable_reference_fails_closed_when_snapshot_is_missing_or_corrupt",
    ),
    "test-snapshot-inventory-remove-pinned-final": ExceptionSpec(
        "dux-core/src/persistence/snapshot/tests.rs",
        "rust-filesystem-effect",
        "test:retention_inventory_rejects_inconsistent_active_pin_storage",
    ),
    "test-snapshot-inventory-oversized-temp": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-truncation-effect",
        "test:inventory_rejects_final_larger_than_codec_limit_but_not_temp_by_policy",
    ),
    "test-snapshot-inventory-oversized-final": ExceptionSpec(
        "dux-core/src/persistence/snapshot/storage/tests.rs",
        "rust-truncation-effect",
        "test:inventory_rejects_final_larger_than_codec_limit_but_not_temp_by_policy",
    ),
    "test-reset-database-writer-helper-spawn": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-process-spawn",
        "test:app_data_reset_deadline_bounds_an_independent_database_writer",
    ),
    "test-reset-cache-stage-replacement-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_data_detach_rejects_replaced_detached_cache_stage",
    ),
    "test-reset-data-root-replacement-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_data_detach_rejects_a_replaced_canonical_root",
    ),
    "test-completed-reset-root-withhold-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_missing_or_replaced_root_refuses_without_provisioning",
    ),
    "test-completed-reset-sentinel-handoff-remove": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_binds_initialized_state_across_store_preparation",
    ),
    "test-completed-reset-root-case-alias-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_rejects_a_case_folded_canonical_root_alias",
    ),
    "test-completed-reset-final-root-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_repeats_root_identity_under_the_final_writer_gate",
    ),
    "test-completed-reset-case-alias-intermediate": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_evolved_root_accepts_a_same_object_case_alias",
    ),
    "test-completed-reset-case-alias-final": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:completed_reset_evolved_root_accepts_a_same_object_case_alias",
    ),
    "test-reset-recovery-impossible-data-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_recovery_refuses_prepared_with_already_detached_data",
    ),
    "test-reset-recovery-impossible-cache-rename": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_recovery_refuses_cache_detached_with_canonical_cache",
    ),
    "test-reset-recovery-observed-journal-remove": ExceptionSpec(
        "dux-core/src/engine/registry_tests.rs",
        "rust-filesystem-effect",
        "test:app_data_reset_recovery_handoff_rejects_a_missing_observed_journal_without_mutation",
    ),
    "test-storage-recovery-alias-rename": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-filesystem-effect",
        "test:recovery_rejects_wrong_identity_layout_alias_and_stage_type",
    ),
    "test-storage-recovery-wrong-type-rename": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-filesystem-effect",
        "test:recovery_rejects_wrong_identity_layout_alias_and_stage_type",
    ),
    "test-storage-root-source-swap": ExceptionSpec(
        "dux-core/src/persistence/storage/windows.rs",
        "rust-filesystem-effect",
        "test:staged_root_source_path_swap_never_publishes_the_replacement",
    ),
    "test-storage-final-root-rename-guard": ExceptionSpec(
        "dux-core/src/persistence/storage/windows.rs",
        "rust-filesystem-effect",
        "test:fresh_store_reopens_before_blocking_external_final_root_rename",
    ),
    "test-storage-cleanup-lock-rename-guard": ExceptionSpec(
        "dux-core/src/persistence/storage.rs",
        "rust-filesystem-effect",
        "test:retained_cleanup_lock_blocks_path_replacement",
    ),
    "build-xcframework-staging-remove": ExceptionSpec(
        "dux-macos/scripts/build-rust-xcframework.sh", "shell-remove"
    ),
    "build-xcframework-output-remove": ExceptionSpec(
        "dux-macos/scripts/build-rust-xcframework.sh", "shell-remove"
    ),
    "build-xcframework-publish-move": ExceptionSpec(
        "dux-macos/scripts/build-rust-xcframework.sh", "shell-move"
    ),
    "build-bindings-staging-remove": ExceptionSpec(
        "dux-macos/scripts/generate-bindings.sh", "shell-remove"
    ),
    "build-bundled-cli-staging-remove": ExceptionSpec(
        "dux-macos/scripts/build-bundled-cli.sh", "shell-remove"
    ),
    "build-bundled-cli-binary-publish": ExceptionSpec(
        "dux-macos/scripts/build-bundled-cli.sh", "shell-move"
    ),
    "build-bundled-cli-metadata-publish": ExceptionSpec(
        "dux-macos/scripts/build-bundled-cli.sh", "shell-move"
    ),
    "build-bundled-cli-final-staging-remove": ExceptionSpec(
        "dux-macos/scripts/build-bundled-cli.sh", "shell-remove"
    ),
    "build-bundled-cli-metadata-inspect": ExceptionSpec(
        "dux-macos/scripts/build-bundled-cli.sh", "shell-indirect-command"
    ),
    "build-bundled-cli-embed-staging-remove": ExceptionSpec(
        "dux-macos/scripts/embed-verified-bundled-cli.sh", "shell-remove"
    ),
    "build-bundled-cli-embed-final-staging-remove": ExceptionSpec(
        "dux-macos/scripts/embed-verified-bundled-cli.sh", "shell-remove"
    ),
    "build-bundled-cli-embed-metadata-inspect": ExceptionSpec(
        "dux-macos/scripts/embed-verified-bundled-cli.sh", "shell-indirect-command"
    ),
    "release-dmg-staging-remove": ExceptionSpec(
        "dux-macos/scripts/release-notarized-dmg.sh", "shell-remove"
    ),
    "release-dmg-publish-move": ExceptionSpec(
        "dux-macos/scripts/release-notarized-dmg.sh", "shell-move"
    ),
    "release-prepared-envelope-publish": ExceptionSpec(
        "dux-macos/scripts/release-notarized-dmg.sh", "shell-move"
    ),
    "release-bundled-cli-manifest-publish": ExceptionSpec(
        "dux-macos/scripts/release-notarized-dmg.sh", "shell-move"
    ),
    "release-bundled-cli-metadata-inspect": ExceptionSpec(
        "dux-macos/scripts/release-notarized-dmg.sh", "shell-indirect-command"
    ),
    "qualification-builder-help-argument": ExceptionSpec(
        "dux-macos/scripts/build-cleanup-qualification-app.sh",
        "shell-indirect-command",
    ),
    "qualification-verifier-help-argument": ExceptionSpec(
        "dux-macos/scripts/verify-signed-cleanup-qualification.sh",
        "shell-indirect-command",
    ),
    "test-public-sparkle-verifier-compile-spawn": ExceptionSpec(
        "scripts/tests/test_macos_release_script.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "test-public-sparkle-verifier-run-spawn": ExceptionSpec(
        "scripts/tests/test_macos_release_script.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "spike-ai-reviewed-child-spawn": ExceptionSpec(
        "spikes/ai-subprocess-confinement/host.c", "c-process-spawn", "c:run_child"
    ),
    "spike-ai-sentinel-cleanup": ExceptionSpec(
        "spikes/ai-subprocess-confinement/host.c",
        "c-filesystem-effect",
        "c:cleanup_owned_root",
    ),
    "spike-ai-private-dir-cleanup": ExceptionSpec(
        "spikes/ai-subprocess-confinement/host.c",
        "c-filesystem-effect",
        "c:cleanup_owned_root",
    ),
    "spike-ai-work-dir-cleanup": ExceptionSpec(
        "spikes/ai-subprocess-confinement/host.c",
        "c-filesystem-effect",
        "c:cleanup_owned_root",
    ),
    "spike-ai-root-cleanup": ExceptionSpec(
        "spikes/ai-subprocess-confinement/host.c",
        "c-filesystem-effect",
        "c:cleanup_owned_root",
    ),
    "macos-cli-installer-publish-new": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-publish-upgrade": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-displaced-unlink": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-stage-unlink": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-upgrade-rollback": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-uninstall": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-uninstall-swap": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-uninstall-displaced": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-uninstall-sentinel-cleanup": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-uninstall-sentinel-failure-cleanup": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "macos-cli-installer-uninstall-sentinel-create-cleanup": ExceptionSpec(
        "dux-macos/Dux/Services/CLIInstallerService.swift",
        "swift-filesystem-effect",
    ),
    "test-cli-installer-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/CLIInstallerServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-cli-installer-fixture-cleanup": ExceptionSpec(
        "dux-macos/DuxTests/CLIInstallerServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-cli-installer-async-fixture-cleanup": ExceptionSpec(
        "dux-macos/DuxTests/CLIInstallerServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-engine-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/EngineServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-storage-roots-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/EngineServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-cleanup-exclusions-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/CleanupExclusionsTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-permanent-policy-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/PermanentCleanupPolicyServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-cargo-inspection-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/DirectCargoEnrollmentTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-retry-obstruction-remove": ExceptionSpec(
        "dux-macos/DuxTests/EngineServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-home-scan-fixture-write": ExceptionSpec(
        "dux-macos/DuxTests/EngineServiceTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-project-roots-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/ProjectDiscoveryRootsTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "test-swift-icloud-reader-fixture-remove": ExceptionSpec(
        "dux-macos/DuxTests/ICloudLocalCopyRawFactReaderTests.swift",
        "swift-filesystem-effect",
        "test",
    ),
    "macos-trash-platform-adapter": ExceptionSpec(
        "dux-macos/Dux/Services/ExplorerLiveFileActions.swift",
        "swift-filesystem-effect",
    ),
    "release-checksum-sidecars-remove": ExceptionSpec(
        ".github/workflows/release.yml", "shell-remove"
    ),
    "macos-release-import-secret-remove": ExceptionSpec(
        ".github/workflows/release-macos-app.yml", "shell-remove"
    ),
    "lint-list-repository-sources": ExceptionSpec(
        "scripts/check_destructive_calls.py", "python-filesystem-or-process-effect"
    ),
    "test-lint-rejected-xcframework-output": ExceptionSpec(
        "scripts/tests/test_check_destructive_calls.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "test-lint-symlinked-xcframework-parent": ExceptionSpec(
        "scripts/tests/test_check_destructive_calls.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "test-lint-fixture-git-init": ExceptionSpec(
        "scripts/tests/test_check_destructive_calls.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "test-lint-fixture-git-add": ExceptionSpec(
        "scripts/tests/test_check_destructive_calls.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "test-release-script-spawn": ExceptionSpec(
        "scripts/tests/test_macos_release_script.py",
        "python-filesystem-or-process-effect",
        "test",
    ),
    "test-ffi-cleanup-success-binary": ExceptionSpec(
        "scripts/test_ffi_rust_target_cleanup.sh",
        "shell-indirect-command",
    ),
    "test-ffi-cleanup-refusal-binary": ExceptionSpec(
        "scripts/test_ffi_rust_target_cleanup.sh",
        "shell-indirect-command",
    ),
}

EXCEPTION_PRIMITIVES = {
    "finder-reveal": "Command::new",
    "cargo-metadata-observer-spawn": "Command::new",
    "cargo-suspended-observer-spawn": "posix_spawn",
    "test-cargo-executable-ancestor-rename": "rename",
    "test-cargo-workspace-ancestor-rename": "rename",
    "test-cargo-closed-stdio-helper-spawn": "Command::new",
    "test-cargo-config-transient-remove": "remove_file",
    "test-target-namespace-ghost-remove": "remove_file",
    "test-target-namespace-legacy-remove": "remove_file",
    "test-target-namespace-source-remove": "remove_file",
    "test-target-namespace-transient-remove": "remove_file",
    "test-workspace-glob-member-remove": "remove_dir",
    "test-workspace-glob-transient-remove": "remove_dir",
    "test-manifest-probe-alias-replace": "remove_file",
    "test-manifest-probe-ancestor-rename": "rename",
    "test-manifest-probe-replacement-remove": "remove_dir",
    "test-manifest-probe-ancestor-restore": "rename",
    "test-manifest-probe-unrelated-remove": "remove_file",
    "test-manifest-probe-absent-remove": "remove_file",
    "test-cargo-cwd-replace": "rename",
    "test-cli-inspection-spawn": "Command::new",
    "test-persistence-helper-spawn": "Command::new",
    "test-capacity-cross-process-helper-spawn": "Command::new",
    "test-persistence-displace-shm": "rename",
    "test-persistence-truncate-owned-database": "set_len",
    "permanent-safe-rust-target-descriptor-contents": "unlinkat",
    "test-reviewed-trash-replaced-file-remove": "remove_file",
    "test-reviewed-trash-missing-file-remove": "remove_file",
    "test-trash-admission-target-drift-remove": "remove_file",
    "test-cloud-probe-replace-rename": "rename",
    "test-cloud-probe-callback-replace-rename": "rename",
    "test-snapshot-diff-removed-rename": "rename",
    "test-snapshot-diff-replaced-rename": "rename",
    "test-ffi-snapshot-diff-removed-rename": "rename",
    "test-descendant-policy-replacement-rename": "rename",
    "test-rule-scope-target-replacement-rename": "rename",
    "test-exact-review-replacement-remove-temp": "remove_dir",
    "test-exact-review-replacement-rename-away": "rename",
    "test-exact-review-replacement-remove-root": "remove_dir_all",
    "test-exact-review-replacement-rename-back": "rename",
    "cache-write-failure-temp-remove": "remove_file",
    "cache-atomic-publish": "rename",
    "cache-publish-failure-temp-remove": "remove_file",
    "test-cache-first-temp-remove": "remove_file",
    "test-cache-second-temp-remove": "remove_file",
    "test-cache-reset-lock-helper-spawn": "Command::new",
    "test-data-namespace-publication-helper-spawn": "Command::new",
    "test-cache-reset-canonical-binding-rename": "rename",
    "test-protected-replaced-root": "rename",
    "test-engine-move-scan-root": "rename",
    "test-subtree-traversal-root-rename": "rename",
    "test-subtree-publication-root-rename": "rename",
    "test-targeted-scan-root-replacement-rename": "rename",
    "test-live-target-replace-rename": "rename",
    "test-live-target-missing-remove": "remove_file",
    "test-live-target-symlink-ancestor-rename": "rename",
    "test-candidate-recovery-missing-snapshot-rename": "rename",
    "test-durable-rust-target-replace-rename": "rename",
    "test-durable-rust-project-replace-rename": "rename",
    "test-durable-rust-target-move-back": "rename",
    "test-durable-rust-manifest-move-back": "rename",
    "test-reset-old-store-nonmonotonic-control-remove": "remove_file",
    "storage-root-handle-publish": "SetFileInformationByHandle",
    "storage-root-linux-publish": "SYS_renameat2",
    "storage-root-macos-publish": "renameatx_np",
    "app-data-reset-old-database-payload-unlink": "unlinkat",
    "app-data-reset-old-database-control-unlink": "unlinkat",
    "app-data-reset-fresh-origin-unlink": "unlinkat",
    "app-data-reset-old-database-root-rmdir": "unlinkat",
    "snapshot-linux-no-replace-publish": "SYS_renameat2",
    "snapshot-macos-no-replace-publish": "renameatx_np",
    "snapshot-current-temp-unlink": "unlinkat",
    "snapshot-observed-final-unlink": "unlinkat",
    "app-data-reset-snapshot-control-unlink": "unlinkat",
    "app-data-reset-snapshot-directory-unlink": "unlinkat",
    "snapshot-provisioning-stage-control-unlink": "unlinkat",
    "snapshot-provisioning-stage-rmdir": "unlinkat",
    "snapshot-windows-current-temp-delete": "SetFileInformationByHandle",
    "snapshot-windows-observed-final-delete": "SetFileInformationByHandle",
    "snapshot-windows-provisioning-stage-delete": "SetFileInformationByHandle",
    "snapshot-windows-handle-publish": "SetFileInformationByHandle",
    "managed-cache-entry-publish": "renameat",
    "managed-cache-exact-clear": "unlinkat",
    "managed-cache-exact-stage-remove": "unlinkat",
    "managed-cache-reset-stage-control-unlink": "unlinkat",
    "managed-cache-reset-stage-rmdir": "unlinkat",
    "managed-cache-linux-store-publish": "SYS_renameat2",
    "managed-cache-macos-store-publish": "renameatx_np",
    "app-data-reset-linux-coordinator-publish": "SYS_renameat2",
    "app-data-reset-macos-coordinator-publish": "renameatx_np",
    "app-data-reset-provisioning-stage-reconcile": "unlinkat",
    "app-data-reset-journal-publish": "renameat",
    "app-data-reset-exact-file-unlink": "unlinkat",
    "test-snapshot-lock-helper-spawn": "Command::new",
    "test-snapshot-reset-canonical-binding-rename": "rename",
    "test-snapshot-temp-lock-helper-spawn": "Command::new",
    "test-snapshot-durable-temp-helper-spawn": "Command::new",
    "test-snapshot-inventory-fd-helper-spawn": "Command::new",
    "test-snapshot-umask-helper-spawn": "Command::new",
    "test-snapshot-macos-parent-acl-command": "Command::new",
    "test-snapshot-macos-final-acl-command": "Command::new",
    "test-snapshot-provisioning-stage-fixture-control-reset": "remove_file",
    "test-snapshot-provisioning-stage-fixture-directory-reset": "remove_dir",
    "test-snapshot-provisioning-stage-file-reset": "remove_file",
    "test-snapshot-provisioning-stage-extra-reset": "remove_file",
    "test-reset-snapshot-marker-only-fixture": "remove_file",
    "test-reset-snapshot-case-alias-intermediate-rename": "rename",
    "test-reset-snapshot-case-alias-final-rename": "rename",
    "test-reset-snapshot-case-alias-restore": "rename",
    "test-snapshot-referenced-file-remove": "remove_file",
    "test-snapshot-inventory-remove-pinned-final": "remove_file",
    "test-snapshot-inventory-oversized-temp": "set_len",
    "test-snapshot-inventory-oversized-final": "set_len",
    "test-reset-database-writer-helper-spawn": "Command::new",
    "test-reset-cache-stage-replacement-rename": "rename",
    "test-reset-data-root-replacement-rename": "rename",
    "test-completed-reset-root-withhold-rename": "rename",
    "test-completed-reset-case-alias-intermediate": "rename",
    "test-completed-reset-case-alias-final": "rename",
    "test-storage-root-source-swap": "rename",
    "test-storage-final-root-rename-guard": "rename",
    "test-storage-cleanup-lock-rename-guard": "rename",
    "build-xcframework-staging-remove": "rm",
    "build-xcframework-output-remove": "rm",
    "build-xcframework-publish-move": "mv",
    "build-bindings-staging-remove": "rm",
    "build-bundled-cli-staging-remove": "rm",
    "build-bundled-cli-binary-publish": "mv",
    "build-bundled-cli-metadata-publish": "mv",
    "build-bundled-cli-final-staging-remove": "rm",
    "build-bundled-cli-metadata-inspect": "$staged_binary",
    "build-bundled-cli-embed-staging-remove": "rm",
    "build-bundled-cli-embed-final-staging-remove": "rm",
    "build-bundled-cli-embed-metadata-inspect": "$binary",
    "release-dmg-staging-remove": "rm",
    "release-dmg-publish-move": "mv",
    "release-prepared-envelope-publish": "mv",
    "release-bundled-cli-manifest-publish": "mv",
    "release-bundled-cli-metadata-inspect": "$cli",
    "spike-ai-reviewed-child-spawn": "posix_spawn",
    "spike-ai-sentinel-cleanup": "unlink",
    "spike-ai-private-dir-cleanup": "unlink",
    "spike-ai-work-dir-cleanup": "unlink",
    "spike-ai-root-cleanup": "unlink",
    "macos-cli-installer-publish-new": "renameatx_np",
    "macos-cli-installer-publish-upgrade": "renameatx_np",
    "macos-cli-installer-displaced-unlink": "unlinkat",
    "macos-cli-installer-stage-unlink": "unlinkat",
    "macos-cli-installer-upgrade-rollback": "renameatx_np",
    "macos-cli-installer-uninstall": "unlinkat",
    "macos-cli-installer-uninstall-swap": "renameatx_np",
    "macos-cli-installer-uninstall-displaced": "unlinkat",
    "macos-cli-installer-uninstall-sentinel-cleanup": "unlinkat",
    "macos-cli-installer-uninstall-sentinel-failure-cleanup": "unlinkat",
    "macos-cli-installer-uninstall-sentinel-create-cleanup": "unlinkat",
    "test-cli-installer-fixture-remove": "removeItem",
    "test-cli-installer-fixture-cleanup": "removeItem",
    "test-cli-installer-async-fixture-cleanup": "removeItem",
    "test-swift-engine-fixture-remove": "removeItem",
    "test-swift-storage-roots-fixture-remove": "removeItem",
    "test-swift-cleanup-exclusions-fixture-remove": "removeItem",
    "test-swift-permanent-policy-fixture-remove": "removeItem",
    "test-swift-cargo-inspection-fixture-remove": "removeItem",
    "test-swift-retry-obstruction-remove": "removeItem",
    "test-swift-home-scan-fixture-write": "write",
    "test-project-roots-fixture-remove": "removeItem",
    "test-swift-icloud-reader-fixture-remove": "removeItem",
    "macos-trash-platform-adapter": "trashItem",
    "release-checksum-sidecars-remove": "rm",
    "macos-release-import-secret-remove": "rm",
    "lint-list-repository-sources": "subprocess.run",
    "test-lint-rejected-xcframework-output": "subprocess.run",
    "test-lint-symlinked-xcframework-parent": "subprocess.run",
    "test-lint-fixture-git-init": "subprocess.run",
    "test-lint-fixture-git-add": "subprocess.run",
    "test-release-script-spawn": "subprocess.run",
    "test-ffi-cleanup-success-binary": "test_binary",
    "test-ffi-cleanup-refusal-binary": "test_binary",
}

CLIPPY_SUPPRESSION_COUNTS = {
    "dux-cli/src/app/state.rs": 1,
    "dux-core/src/cache/mod.rs": 2,
    "dux-core/src/cleanup/executor.rs": 1,
    "dux-core/src/path_validation/protected/tests.rs": 1,
    "dux-core/src/planner/descendant_policy.rs": 1,
    "dux-core/src/planner/exact_path_review_tests.rs": 1,
    "dux-core/src/planner/rule_scope_grant.rs": 1,
    "dux-core/src/planner/rust_target_cargo/runner.rs": 1,
    "dux-core/src/planner/cargo_spawn_macos.rs": 2,
    "dux-core/src/planner/cargo_workspace.rs": 1,
    "dux-core/src/planner/cargo_config.rs": 1,
    "dux-core/src/planner/cargo_manifest_probes.rs": 3,
    "dux-core/src/planner/cargo_target_namespace.rs": 2,
    "dux-core/src/planner/cargo_workspace_glob.rs": 2,
    "dux-core/src/planner/rust_target_cargo_tests.rs": 1,
    "dux-core/src/planner/rust_target_source_tests.rs": 2,
    "dux-core/src/cache/managed_store/tests.rs": 7,
    "dux-core/src/engine/registry_tests.rs": 21,
    "dux-ffi/src/lib.rs": 1,
    "dux-core/src/engine/volume_status.rs": 1,
    "dux-core/src/persistence/persistence_tests.rs": 3,
    "dux-core/src/persistence/cleanup_journal/tests.rs": 1,
    "dux-core/src/persistence/storage.rs": 4,
    "dux-core/src/persistence/storage/windows.rs": 2,
    "dux-core/src/persistence/snapshot/storage/app_data_reset.rs": 1,
    "dux-core/src/persistence/snapshot/storage/tests.rs": 12,
    "dux-core/src/persistence/snapshot/tests.rs": 3,
    "dux-cli/tests/inspection_cli.rs": 1,
}

CLIPPY_PRODUCT_SUPPRESSION_SYMBOLS = {
    "dux-cli/src/app/state.rs": {"open_in_finder"},
    "dux-core/src/cache/mod.rs": {"save_cache"},
    "dux-core/src/planner/rust_target_cargo/runner.rs": {"run_cargo_portable"},
    "dux-core/src/persistence/snapshot/storage/app_data_reset.rs": {
        "revalidate_structural_final_gate_until"
    },
}

RETIRED_LEGACY_MODULE = "legacy" + "_cli"
RETIRED_LEGACY_SYMBOL_PREFIX = "Legacy" + "Cli" + "PermanentDelete"


def _retired_legacy_cli_findings(path: str, source: str) -> list[Finding]:
    if pathlib.PurePosixPath(path).suffix.lower() != ".rs" or not path.startswith(
        ("dux-cli/src/", "dux-core/src/", "dux-ffi/src/")
    ):
        return []

    sanitized = _strip_c_like_comments_and_literals(source)
    markers = (
        re.compile(rf"\b{re.escape(RETIRED_LEGACY_MODULE)}\b"),
        re.compile(rf"\b{re.escape(RETIRED_LEGACY_SYMBOL_PREFIX)}[A-Za-z0-9_]*\b"),
    )
    findings = []
    for marker in markers:
        for match in marker.finditer(sanitized):
            findings.append(
                Finding(
                    path,
                    _line_number(sanitized, match.start()),
                    "retired-legacy-cli-architecture",
                    "the retired CLI permanent-delete adapter must not be reintroduced",
                )
            )

    if pathlib.PurePosixPath(path).stem == RETIRED_LEGACY_MODULE and not findings:
        findings.append(
            Finding(
                path,
                1,
                "retired-legacy-cli-architecture",
                "the retired CLI permanent-delete module path must not be reintroduced",
            )
        )
    return findings


def _line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _line_snippet(lines: list[str], line: int) -> str:
    if 1 <= line <= len(lines):
        return lines[line - 1].strip()[:160]
    return ""


def _strip_c_like_comments_and_literals(source: str) -> str:
    """Replace C/Rust/Swift comments and literals while preserving offsets."""

    output = list(source)
    index = 0
    length = len(source)
    raw_string_prefix = re.compile(r'r(#+)?"')

    def blank(start: int, end: int) -> None:
        for position in range(start, end):
            if output[position] != "\n":
                output[position] = " "

    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = length if end == -1 else end
            blank(index, end)
            index = end
            continue
        if source.startswith("/*", index):
            depth = 1
            end = index + 2
            while end < length and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            blank(index, end)
            index = end
            continue
        if source.startswith('"""', index):
            end = source.find('"""', index + 3)
            end = length if end == -1 else end + 3
            blank(index, end)
            index = end
            continue
        raw = raw_string_prefix.match(source, index)
        if raw:
            hashes = raw.group(1) or ""
            delimiter = '"' + hashes
            content_start = index + len(raw.group(0))
            end = source.find(delimiter, content_start)
            end = length if end == -1 else end + len(delimiter)
            blank(index, end)
            index = end
            continue
        if source[index] == '"':
            end = index + 1
            while end < length:
                if source[end] == "\\":
                    end += 2
                    continue
                end += 1
                if source[end - 1] == '"':
                    break
            blank(index, min(end, length))
            index = end
            continue
        index += 1
    return "".join(output)


def _strip_c_like_comments(source: str) -> str:
    output = list(source)
    index = 0
    while index < len(source):
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end == -1 else end
            for position in range(index, end):
                output[position] = " "
            index = end
            continue
        if source.startswith("/*", index):
            depth = 1
            end = index + 2
            while end < len(source) and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            for position in range(index, end):
                if output[position] != "\n":
                    output[position] = " "
            index = end
            continue
        if source[index] in {'"', "'"}:
            quote = source[index]
            index += 1
            while index < len(source):
                if source[index] == "\\":
                    index += 2
                    continue
                if source[index] == quote:
                    index += 1
                    break
                index += 1
            continue
        index += 1
    return "".join(output)


def _strip_shell_comments(source: str) -> str:
    output: list[str] = []
    quote: str | None = None
    index = 0
    while index < len(source):
        character = source[index]
        if character == "\\" and quote != "'" and index + 1 < len(source):
            output.extend((character, source[index + 1]))
            index += 2
            continue
        if character in {"'", '"'}:
            quote = None if quote == character else character if quote is None else quote
            output.append(character)
            index += 1
            continue
        if character == "#" and quote is None:
            end = source.find("\n", index)
            if end == -1:
                output.extend(" " for _ in range(index, len(source)))
                break
            output.extend(" " for _ in range(index, end))
            output.append("\n")
            index = end + 1
            continue
        output.append(character)
        index += 1
    return "".join(output)


def _strip_simple_quoted_literals(source: str) -> str:
    output = list(source)
    quote: str | None = None
    index = 0
    while index < len(source):
        character = source[index]
        if quote is None and character in {"'", '"'}:
            quote = character
            output[index] = " "
        elif quote is not None:
            if character != "\n":
                output[index] = " "
            if character == quote:
                quote = None
        index += 1
    return "".join(output)


def _strip_batch_echo_lines(source: str) -> str:
    output = []
    for line in source.splitlines(keepends=True):
        if re.match(r"(?i)^\s*@?echo(?:\s|$)", line):
            output.append("".join("\n" if character == "\n" else " " for character in line))
        else:
            output.append(line)
    return "".join(output)


def _regex_matches(source: str, lines: list[str], rules: Iterable[tuple[str, re.Pattern[str]]]) -> list[Match]:
    matches: list[Match] = []
    for rule, pattern in rules:
        for match in pattern.finditer(source):
            line = _line_number(source, match.start())
            matches.append(Match(line, rule, _line_snippet(lines, line), match.group(0)))
    return matches


def _shell_tokens(command: str) -> list[str]:
    lexer = shlex.shlex(command, posix=True, punctuation_chars=";&|()")
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return list(lexer)


def _shell_rule_for_tokens(tokens: list[str], aliases: dict[str, str] | None = None) -> str | None:
    while tokens and tokens[0] in {"-", "!", "if", "then", "do", "while", "until", "{"}:
        tokens = tokens[1:]
    if tokens and tokens[0] == "run:":
        tokens = tokens[1:]
    while tokens and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", tokens[0]):
        tokens = tokens[1:]
    if tokens and pathlib.PurePosixPath(tokens[0]).name == "command" and "-v" in tokens[1:]:
        return None
    while tokens and pathlib.PurePosixPath(tokens[0]).name in {
        "command",
        "sudo",
        "env",
        "exec",
        "nohup",
        "nice",
        "time",
    }:
        tokens = tokens[1:]
        while tokens and (tokens[0].startswith("-") or "=" in tokens[0]):
            tokens = tokens[1:]
    if not tokens:
        return None
    command_token = tokens[0]
    alias_name = command_token.removeprefix("$").strip("{}")
    if aliases and alias_name in aliases:
        command_token = aliases[alias_name]
    elif aliases and command_token in aliases:
        command_token = aliases[command_token]
    elif command_token.startswith("$") and not command_token.startswith("$SCRIPT_DIR/"):
        return "shell-indirect-command"
    command = pathlib.PurePosixPath(command_token).name
    if command in {"rm", "rmdir", "unlink"}:
        return "shell-remove"
    if command == "mv":
        return "shell-move"
    if command == "truncate":
        return "shell-truncate"
    if command == "git" and "clean" in tokens[1:]:
        return "shell-git-clean"
    if command == "find" and any(
        token in {"-delete", "-exec", "-execdir"}
        or pathlib.PurePosixPath(token).name in {"rm", "rmdir", "unlink"}
        for token in tokens[1:]
    ):
        return "shell-find-delete"
    if command == "xargs":
        return "shell-process-spawn"
    if command == "eval":
        return "shell-eval"
    if command in {"sh", "bash", "zsh"} and "-c" in tokens[1:]:
        return "shell-command-string"
    if command == "osascript":
        return "shell-command-string"
    return None


def _shell_matches(
    source: str,
    lines: list[str],
    *,
    detect_indirect: bool = True,
) -> list[Match]:
    logical_lines: list[tuple[int, str]] = []
    start_line = 1
    buffer = ""
    for line_number, line in enumerate(lines, start=1):
        if not buffer:
            start_line = line_number
        stripped = line.rstrip()
        if stripped.endswith("\\"):
            buffer += stripped[:-1]
            continue
        logical_lines.append((start_line, buffer + line))
        buffer = ""
    if buffer:
        logical_lines.append((start_line, buffer))

    matches: list[Match] = []
    aliases: dict[str, str] = {}
    separators = {
        ";",
        ";;",
        ";&",
        ";;&",
        "&&",
        "||",
        "&",
        "|",
        "(",
        ")",
        "()",
        "{",
        "}",
    }
    case_pattern_expected = False
    for line_number, logical in logical_lines:
        try:
            tokens = _shell_tokens(logical)
        except ValueError:
            matches.extend(_regex_matches(logical, lines, SHELL_RULES))
            continue
        if tokens and tokens[0] == "case" and tokens[-1] == "in":
            case_pattern_expected = True
            continue
        if tokens == ["esac"]:
            case_pattern_expected = False
            continue
        if case_pattern_expected:
            try:
                pattern_end = tokens.index(")")
            except ValueError:
                matches.extend(_regex_matches(logical, lines, SHELL_RULES))
                continue
            tokens = tokens[pattern_end + 1:]
            case_pattern_expected = False
        terminates_case_clause = any(token in {";;", ";&", ";;&"} for token in tokens)
        segments: list[list[str]] = [[]]
        for token in tokens:
            if token in separators:
                segments.append([])
            else:
                segments[-1].append(token)
        for segment in segments:
            if not segment:
                continue
            if len(segment) == 1 and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", segment[0]):
                name, value = segment[0].split("=", 1)
                value_parts = value.split(maxsplit=1)
                value_command = value_parts[0] if value_parts else ""
                if pathlib.PurePosixPath(value_command).name in {"rm", "rmdir", "unlink", "mv", "truncate"}:
                    aliases[name] = value_command
            if segment[0] == "alias" and len(segment) > 1 and "=" in segment[1]:
                name, value = segment[1].split("=", 1)
                value_tokens = _shell_tokens(value)
                while value_tokens and pathlib.PurePosixPath(value_tokens[0]).name in {
                    "command",
                    "sudo",
                    "env",
                    "exec",
                    "nohup",
                    "nice",
                    "time",
                }:
                    value_tokens = value_tokens[1:]
                value_command = value_tokens[0] if value_tokens else ""
                if pathlib.PurePosixPath(value_command).name in {"rm", "rmdir", "unlink", "mv", "truncate"}:
                    aliases[name] = value_command
            rule = _shell_rule_for_tokens(segment, aliases)
            if rule == "shell-indirect-command" and not detect_indirect:
                rule = None
            if rule is not None:
                sanitized_line = _strip_shell_comments(lines[line_number - 1])
                matches.append(Match(line_number, rule, _line_snippet(lines, line_number), sanitized_line))
            command = pathlib.PurePosixPath(segment[0]).name
            nested: str | None = None
            if command == "trap" and len(segment) > 1:
                nested = segment[1]
            elif segment[0] == "shellScript" and len(segment) > 2:
                nested = segment[2]
            if nested:
                try:
                    nested_tokens = _shell_tokens(nested)
                except ValueError:
                    nested_tokens = []
                nested_segments: list[list[str]] = [[]]
                for token in nested_tokens:
                    if token in separators:
                        nested_segments.append([])
                    else:
                        nested_segments[-1].append(token)
                for nested_segment in nested_segments:
                    nested_rule = _shell_rule_for_tokens(nested_segment, aliases)
                    if nested_rule == "shell-indirect-command" and not detect_indirect:
                        nested_rule = None
                    if nested_rule is not None:
                        sanitized_line = _strip_shell_comments(lines[line_number - 1])
                        matches.append(
                            Match(
                                line_number,
                                nested_rule,
                                _line_snippet(lines, line_number),
                                sanitized_line,
                            )
                        )
        if terminates_case_clause:
            case_pattern_expected = True
    return matches


def _python_name(node: ast.AST) -> str:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        prefix = _python_name(node.value)
        return f"{prefix}.{node.attr}" if prefix else node.attr
    return ""


def _python_matches(source: str, lines: list[str]) -> list[Match]:
    try:
        tree = ast.parse(source)
    except SyntaxError as error:
        return [Match(error.lineno or 1, "python-syntax", error.msg)]

    aliases: dict[str, str] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                aliases[alias.asname or alias.name] = alias.name
        elif isinstance(node, ast.ImportFrom) and node.module:
            for alias in node.names:
                aliases[alias.asname or alias.name] = f"{node.module}.{alias.name}"

    containers: dict[str, object] = {}

    def merge_resolved(existing: object, incoming: object) -> object:
        if isinstance(existing, str) and existing in PYTHON_EFFECTS:
            return existing
        if isinstance(incoming, str) and incoming in PYTHON_EFFECTS:
            return incoming
        if isinstance(incoming, dict):
            merged = dict(existing) if isinstance(existing, dict) else {}
            for key, value in incoming.items():
                merged[key] = merge_resolved(merged.get(key, ""), value)
            return merged
        if isinstance(incoming, list):
            merged = list(existing) if isinstance(existing, list) else []
            for index, value in enumerate(incoming):
                if index < len(merged):
                    merged[index] = merge_resolved(merged[index], value)
                else:
                    merged.append(value)
            return merged
        return existing

    def resolve_callable(node: ast.AST) -> object:
        name = _python_name(node)
        prefix, separator, remainder = name.partition(".")
        if prefix in aliases:
            name = aliases[prefix] + (separator + remainder if separator else "")
        if name in PYTHON_EFFECTS:
            return name
        if (
            isinstance(node, ast.Call)
            and _python_name(node.func) in {"getattr", "builtins.getattr"}
            and len(node.args) >= 2
            and isinstance(node.args[1], ast.Constant)
            and isinstance(node.args[1].value, str)
        ):
            owner = _python_name(node.args[0])
            owner = aliases.get(owner, owner)
            qualified = f"{owner}.{node.args[1].value}"
            return qualified if qualified in PYTHON_EFFECTS else ""
        if isinstance(node, ast.Dict):
            return {
                key.value: resolve_callable(value)
                for key, value in zip(node.keys, node.values, strict=True)
                if isinstance(key, ast.Constant)
            }
        if isinstance(node, (ast.List, ast.Tuple)):
            return [resolve_callable(value) for value in node.elts]
        if isinstance(node, ast.Subscript) and isinstance(node.value, ast.Name):
            container = containers.get(node.value.id)
            if isinstance(node.slice, ast.Constant):
                try:
                    return container[node.slice.value]  # type: ignore[index]
                except (KeyError, IndexError, TypeError):
                    return ""
        return ""

    changed = True
    while changed:
        changed = False
        for node in ast.walk(tree):
            if not isinstance(node, (ast.Assign, ast.AnnAssign)):
                continue
            value = node.value
            if value is None:
                continue
            resolved = resolve_callable(value)
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            for target in targets:
                if not isinstance(target, ast.Name):
                    continue
                if isinstance(resolved, str) and resolved in PYTHON_EFFECTS:
                    if aliases.get(target.id) not in PYTHON_EFFECTS:
                        aliases[target.id] = resolved
                        changed = True
                elif isinstance(resolved, (dict, list)):
                    merged = merge_resolved(containers.get(target.id, ""), resolved)
                    if containers.get(target.id) != merged:
                        containers[target.id] = merged
                        changed = True

    matches: list[Match] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            name = _python_name(node.func)
            prefix, separator, remainder = name.partition(".")
            if prefix in aliases:
                name = aliases[prefix] + (separator + remainder if separator else "")
            resolved_func = resolve_callable(node.func)
            if isinstance(resolved_func, str) and resolved_func in PYTHON_EFFECTS:
                name = resolved_func
            attribute = name.rsplit(".", 1)[-1]
            is_effect = name in PYTHON_EFFECTS
            is_effect |= attribute in PYTHON_EFFECT_ATTRIBUTES
            is_effect |= attribute in PYTHON_PROCESS_ATTRIBUTES and (
                name.startswith("subprocess.") or name.startswith("os.")
            )
            if is_effect:
                matches.append(
                    Match(
                        node.lineno,
                        "python-filesystem-or-process-effect",
                        _line_snippet(lines, node.lineno),
                        name,
                    )
                )
            if (
                name in {"getattr", "builtins.getattr"}
                and node.args
                and aliases.get(_python_name(node.args[0]), _python_name(node.args[0]))
                in {"os", "shutil", "subprocess", "pathlib"}
                and (len(node.args) < 2 or not isinstance(node.args[1], ast.Constant))
            ):
                matches.append(
                    Match(node.lineno, "python-reflective-effect-access", _line_snippet(lines, node.lineno))
                )
            if name in {"open", "builtins.open", "pathlib.Path.open"}:
                mode_index = 1 if name in {"open", "builtins.open"} else 0
                mode: str | None = None
                if len(node.args) > mode_index and isinstance(node.args[mode_index], ast.Constant):
                    mode = node.args[mode_index].value
                for keyword in node.keywords:
                    if keyword.arg == "mode" and isinstance(keyword.value, ast.Constant):
                        mode = keyword.value.value
                if isinstance(mode, str) and any(flag in mode for flag in "wax+"):
                    matches.append(
                        Match(
                            node.lineno,
                            "python-truncation-effect",
                            _line_snippet(lines, node.lineno),
                        )
                    )
    return matches


def _annotations(lines: list[str]) -> tuple[dict[int, AllowAnnotation], list[Finding]]:
    annotations: dict[int, AllowAnnotation] = {}
    findings: list[Finding] = []
    for line_number, line in enumerate(lines, start=1):
        stripped = line.strip()
        is_comment = stripped.startswith("//") or (
            stripped.startswith("#") and not stripped.startswith("#!")
        )
        if ANNOTATION not in line or not is_comment:
            continue
        match = ANNOTATION_RE.search(line)
        if not match:
            findings.append(Finding("", line_number, "invalid-annotation", "malformed destructive-call annotation"))
            continue
        reason = match.group("reason").strip()
        if len(reason) < MIN_REASON_LENGTH:
            findings.append(
                Finding("", line_number, "invalid-annotation", "annotation reason is too short to be reviewable")
            )
            continue
        annotations[line_number] = AllowAnnotation(
            line_number,
            match.group("exception_id"),
            reason,
        )
    return annotations, findings


def _previous_nonblank(lines: list[str], line: int) -> int | None:
    candidate = line - 1
    while candidate > 0 and not lines[candidate - 1].strip():
        candidate -= 1
    return candidate or None


def _matching_brace(source: str, opening: int) -> int | None:
    depth = 0
    for offset in range(opening, len(source)):
        if source[offset] == "{":
            depth += 1
        elif source[offset] == "}":
            depth -= 1
            if depth == 0:
                return offset
    return None


def _rust_test_ranges(source: str) -> list[tuple[int, int]]:
    sanitized = _strip_c_like_comments_and_literals(source)
    pattern = re.compile(
        r"#\s*\[\s*(?:test|cfg\s*\(\s*test\s*\))\s*\]"
        r"[^{}]{0,1000}\{",
        re.DOTALL,
    )
    ranges = []
    for match in pattern.finditer(sanitized):
        opening = match.end() - 1
        closing = _matching_brace(sanitized, opening)
        if closing is not None:
            ranges.append((match.start(), closing + 1))
    return ranges


def _named_rust_item_range(source: str, name: str) -> tuple[int, int] | None:
    sanitized = _strip_c_like_comments_and_literals(source)
    match = re.search(rf"\bfn\s+{re.escape(name)}\b[^{{;]*\{{", sanitized)
    if match is None:
        return None
    opening = match.end() - 1
    closing = _matching_brace(sanitized, opening)
    return None if closing is None else (match.start(), closing + 1)


def _named_c_item_range(source: str, name: str) -> tuple[int, int] | None:
    sanitized = _strip_c_like_comments_and_literals(source)
    match = re.search(
        rf"(?m)^[A-Za-z_][^;{{}}]{{0,1000}}\b{re.escape(name)}\s*"
        r"\([^;{}]*\)\s*\{",
        sanitized,
    )
    if match is None:
        return None
    opening = match.end() - 1
    closing = _matching_brace(sanitized, opening)
    return None if closing is None else (match.start(), closing + 1)


def _clippy_suppression_findings(
    path: str,
    source: str,
    *,
    enforce_count: bool,
) -> list[Finding]:
    pattern = re.compile(
        r"#\s*!?\[[^\]]{0,1000}\bclippy\s*::\s*disallowed_methods\b",
        re.DOTALL,
    )
    matches = list(pattern.finditer(_strip_c_like_comments_and_literals(source)))
    expected_count = CLIPPY_SUPPRESSION_COUNTS.get(path, 0)
    findings: list[Finding] = []
    if (enforce_count or expected_count == 0) and len(matches) != expected_count:
        findings.append(
            Finding(
                path,
                1,
                "clippy-suppression-drift",
                f"expected {expected_count} reviewed disallowed-method suppressions, found {len(matches)}",
            )
        )
    test_ranges = _rust_test_ranges(source)
    product_ranges = [
        item_range
        for symbol in CLIPPY_PRODUCT_SUPPRESSION_SYMBOLS.get(path, set())
        if (item_range := _named_rust_item_range(source, symbol)) is not None
    ]
    for match in matches:
        allowed = any(start <= match.start() < end for start, end in test_ranges)
        allowed |= any(start <= match.start() < end for start, end in product_ranges)
        allowed |= any(
            0 < start - match.start() <= 500
            and "fn " not in source[match.end() : start]
            for start, _ in product_ranges
        )
        if not allowed:
            findings.append(
                Finding(
                    path,
                    _line_number(source, match.start()),
                    "invalid-clippy-suppression",
                    "disallowed-method suppression is outside the exact reviewed context",
                )
            )
    return findings


def _exception_allowed(
    path: str,
    annotation: AllowAnnotation,
    source: str,
    match: Match,
) -> bool:
    spec = EXCEPTIONS.get(annotation.exception_id)
    if spec is None or spec.path != path or spec.rule != match.rule:
        return False
    primitive = EXCEPTION_PRIMITIVES.get(annotation.exception_id)
    if primitive is not None and primitive not in match.matched:
        return False
    if annotation.exception_id == "finder-reveal" and not re.search(
        r"Command\s*::\s*new\s*\(\s*\"open\"\s*\)\s*\.\s*arg\s*\(\s*\"-R\"\s*\)",
        _strip_c_like_comments(source)[item_range[0] : item_range[1]]
        if (item_range := _named_rust_item_range(source, "open_in_finder")) is not None
        else "",
        re.DOTALL,
    ):
        return False
    if annotation.exception_id == "app-data-reset-fresh-origin-unlink":
        item_range = _named_rust_item_range(
            source, "unlink_app_data_reset_fresh_origin_with_before_unlink"
        )
        if item_range is None:
            return False
        item = _strip_c_like_comments(source[item_range[0] : item_range[1]])
        fixed_name_bindings = list(
            re.finditer(
                r"\blet\s+name\s*=\s*OsStr\s*::\s*new\s*\(\s*"
                r"APP_DATA_RESET_FRESH_ORIGIN_NAME\s*\)\s*;",
                item,
            )
        )
        name_let_bindings = re.findall(r"\blet\b[^;=]*\bname\b[^;=]*=", item)
        closure_bindings = re.findall(r"\|([^|]*)\|", item, re.DOTALL)
        if (
            re.search(r"\bname\s*:", item)
            or len(fixed_name_bindings) != 1
            or len(name_let_bindings) != 1
            or re.search(r"\bfor\b[^;{}]*\bname\b[^;{}]*\bin\b", item)
            or any(re.search(r"\bname\b", binding) for binding in closure_bindings)
            or re.search(
                r"(?:\([^)]*\bname\b[^)]*\)|\bname\b)\s*=>", item, re.DOTALL
            )
        ):
            return False
        final_effect = re.search(
            r"\bbefore_unlink\s*\(\s*\)[^;]*\?\s*;\s*"
            r"unlinkat\s*\(\s*directory\s*,\s*name\s*,\s*"
            r"UnlinkatFlags\s*::\s*NoRemoveDir\s*\)",
            item,
            re.DOTALL,
        )
        if final_effect is None or fixed_name_bindings[0].end() >= final_effect.start():
            return False
    line_offset = sum(len(line) + 1 for line in source.splitlines()[: match.line - 1])
    if spec.context == "test":
        if "/tests/" in f"/{path}" or path.endswith("Tests.swift"):
            return True
        if pathlib.PurePosixPath(path).suffix.lower() != ".rs":
            return False
        return any(start <= line_offset < end for start, end in _rust_test_ranges(source))
    if spec.context.startswith("test:"):
        symbol = spec.context.removeprefix("test:")
        item_range = _named_rust_item_range(source, symbol)
        if item_range is None or not (item_range[0] <= line_offset < item_range[1]):
            return False
        if "/tests/" in f"/{path}":
            return True
        return any(start <= item_range[0] < end for start, end in _rust_test_ranges(source))
    if spec.context.startswith("c:"):
        item_range = _named_c_item_range(source, spec.context.removeprefix("c:"))
        return item_range is not None and item_range[0] <= line_offset < item_range[1]
    if spec.context != "any":
        item_range = _named_rust_item_range(source, spec.context)
        return item_range is not None and item_range[0] <= line_offset < item_range[1]
    return True


def scan_source(
    path: str,
    source: str,
    *,
    executable: bool = False,
    enforce_suppression_count: bool = False,
) -> list[Finding]:
    lines = source.splitlines()
    suffix = pathlib.PurePosixPath(path).suffix.lower()
    annotations, annotation_findings = _annotations(lines)
    findings = [dataclasses.replace(finding, path=path) for finding in annotation_findings]
    findings.extend(_retired_legacy_cli_findings(path, source))
    if suffix == ".rs":
        findings.extend(
            _clippy_suppression_findings(
                path,
                source,
                enforce_count=enforce_suppression_count,
            )
        )

    if suffix == ".rs":
        matches = _regex_matches(_strip_c_like_comments_and_literals(source), lines, RUST_RULES)
    elif suffix == ".swift":
        matches = _regex_matches(_strip_c_like_comments_and_literals(source), lines, SWIFT_RULES)
    elif suffix in {".c", ".cc", ".cpp", ".m", ".mm"}:
        matches = _regex_matches(
            _strip_c_like_comments_and_literals(source),
            lines,
            C_FAMILY_RULES,
        )
    elif suffix in {
        ".sh",
        ".bash",
        ".zsh",
        ".yml",
        ".yaml",
        ".command",
        ".pbxproj",
    }:
        matches = _shell_matches(
            source,
            lines,
            detect_indirect=suffix in {".sh", ".bash", ".zsh", ".command"},
        )
    elif suffix in {".ps1", ".bat", ".cmd"}:
        sanitized = _strip_shell_comments(source)
        if suffix == ".ps1":
            sanitized = _strip_simple_quoted_literals(sanitized)
        else:
            sanitized = _strip_batch_echo_lines(sanitized)
        matches = _regex_matches(sanitized, lines, SHELL_RULES)
    elif suffix == ".py":
        matches = _python_matches(source, lines)
    else:
        first_line = lines[0] if lines else ""
        basename = pathlib.PurePosixPath(path).name
        if "python" in first_line and first_line.startswith("#!"):
            matches = _python_matches(source, lines)
        elif (
            first_line.startswith("#!")
            and any(shell in first_line for shell in ("/sh", "/bash", "/zsh", "/env sh", "/env bash", "/env zsh"))
        ) or basename in {"Makefile", "GNUmakefile"}:
            matches = _shell_matches(source, lines)
        elif executable:
            findings.append(
                Finding(path, 1, "unsupported-executable-language", "executable source language is not classified")
            )
            return findings
        else:
            return findings

    used_annotations: dict[int, int] = {}
    for match in sorted(matches, key=lambda item: (item.line, item.rule)):
        candidate_lines = [match.line]
        previous = _previous_nonblank(lines, match.line)
        if previous is not None:
            candidate_lines.append(previous)
        annotation = next((annotations.get(line) for line in candidate_lines if line in annotations), None)
        if annotation is None:
            findings.append(
                Finding(
                    path,
                    match.line,
                    match.rule,
                    f"unreviewed destructive/process call: {match.snippet}",
                )
            )
            continue
        if not _exception_allowed(path, annotation, source, match):
            findings.append(
                Finding(
                    path,
                    annotation.line,
                    "invalid-annotation-scope",
                    f"exception {annotation.exception_id!r} is unknown or not allowed for this path/rule/context",
                )
            )
            continue
        used_annotations[annotation.line] = used_annotations.get(annotation.line, 0) + 1

    for line, count in used_annotations.items():
        if count > 1:
            findings.append(
                Finding(path, line, "overbroad-annotation", "one annotation may cover only one matched call")
            )
    used_ids: dict[str, list[int]] = {}
    for line in used_annotations:
        used_ids.setdefault(annotations[line].exception_id, []).append(line)
    for exception_id, lines_for_id in used_ids.items():
        if len(lines_for_id) > 1:
            findings.append(
                Finding(
                    path,
                    lines_for_id[1],
                    "duplicate-exception-id",
                    f"exception {exception_id!r} may appear exactly once",
                )
            )
    for line in sorted(set(annotations) - set(used_annotations)):
        findings.append(Finding(path, line, "unused-annotation", "annotation does not guard a matched call"))
    return findings


def _tracked_sources(repo_root: pathlib.Path) -> list[pathlib.Path]:
    # DUX-DESTRUCTIVE: allow=lint-list-repository-sources -- fixed git query only enumerates reviewed repository source paths
    result = subprocess.run(
        [
            "git",
            "-C",
            str(repo_root),
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        check=True,
        capture_output=True,
    )
    paths = []
    for raw_path in result.stdout.split(b"\0"):
        if not raw_path:
            continue
        relative = pathlib.PurePosixPath(raw_path.decode("utf-8"))
        path = repo_root.joinpath(*relative.parts)
        if not path.is_file():
            continue
        prefix = path.read_bytes()[:128]
        is_shebang = prefix.startswith(b"#!")
        is_executable = bool(path.stat().st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))
        if (
            relative.suffix.lower() in SCANNED_SUFFIXES
            or relative.name in {"Makefile", "GNUmakefile"}
            or is_shebang
            or is_executable
        ):
            paths.append(path)
    return sorted(set(paths))


def scan_repository(
    repo_root: pathlib.Path,
    *,
    enforce_registry: bool = True,
) -> tuple[list[Finding], int]:
    findings: list[Finding] = []
    paths = _tracked_sources(repo_root)
    observed_ids: dict[str, list[tuple[str, int]]] = {}
    for path in paths:
        relative = path.relative_to(repo_root).as_posix()
        source = path.read_text(encoding="utf-8")
        executable = bool(path.stat().st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))
        findings.extend(
            scan_source(
                relative,
                source,
                executable=executable,
                enforce_suppression_count=enforce_registry,
            )
        )
        annotations, _ = _annotations(source.splitlines())
        for annotation in annotations.values():
            observed_ids.setdefault(annotation.exception_id, []).append((relative, annotation.line))
    if enforce_registry:
        for exception_id, spec in EXCEPTIONS.items():
            uses = observed_ids.get(exception_id, [])
            if len(uses) != 1:
                findings.append(
                    Finding(
                        spec.path,
                        1,
                        "exception-registry-drift",
                        f"registered exception {exception_id!r} must appear exactly once, found {len(uses)}",
                    )
                )
    for exception_id, uses in observed_ids.items():
        if exception_id in EXCEPTIONS:
            continue
        for path, line in uses:
            findings.append(
                Finding(path, line, "unknown-exception-id", f"unregistered exception {exception_id!r}")
            )
    return sorted(findings, key=lambda finding: (finding.path, finding.line, finding.rule)), len(paths)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parent.parent,
    )
    args = parser.parse_args()
    findings, count = scan_repository(args.repo_root.resolve())
    if findings:
        print("Forbidden destructive-call boundary violations:", file=sys.stderr)
        for finding in findings:
            print(f"  {finding.render()}", file=sys.stderr)
        print(
            f"\nRegister one stable exception ID and use {ANNOTATION}: allow=<id> -- <specific reason>; "
            "annotations are one-use, line-scoped, rule-scoped, and path-restricted.",
            file=sys.stderr,
        )
        return 1
    print(f"Destructive-call boundary clean ({count} repository source files scanned).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
