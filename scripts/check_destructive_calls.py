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
            r"\s*::\s*(?:remove|unlink|unlinkat|rmdir)\b"
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
            r"(?<![A-Za-z0-9_])(?:system|popen|exec[a-z0-9_]*|posix_spawn[a-z0-9_]*|"
            r"CreateProcess[AW]?|ShellExecute[AW]?)\s*\("
            r"|=\s*&?\s*(?:system|popen|exec[a-z0-9_]*|posix_spawn[a-z0-9_]*|"
            r"CreateProcess[AW]?|ShellExecute[AW]?)\b"
            r"|(?m:^\s*#\s*define\s+[A-Za-z_][A-Za-z0-9_]*\s+"
            r"(?:system|popen|exec[a-z0-9_]*|posix_spawn[a-z0-9_]*|"
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
    "legacy-adapter-delete-directory": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "execute_plan"
    ),
    "legacy-adapter-delete-windows-link": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "execute_plan"
    ),
    "legacy-adapter-delete-file": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "execute_plan"
    ),
    "test-delete-replaced-file": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "test:replaced_file_is_not_deleted"
    ),
    "test-delete-replaced-directory": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "test:replaced_directory_is_not_deleted"
    ),
    "test-delete-missing-entry": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "test:missing_entry_is_not_counted_as_deleted"
    ),
    "test-delete-replaced-symlink": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "test:replaced_symlink_is_not_deleted"
    ),
    "test-delete-changed-evidence": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "test:changed_artifact_evidence_blocks_delete"
    ),
    "test-delete-replaced-ancestor": ExceptionSpec(
        "dux-core/src/cleanup/legacy_cli.rs", "rust-filesystem-effect", "test:replaced_ancestor_cannot_redirect_delete_outside_scan_root"
    ),
    "finder-reveal": ExceptionSpec("dux-cli/src/app/state.rs", "rust-process-spawn", "open_in_finder"),
    "test-persistence-helper-spawn": ExceptionSpec(
        "dux-core/src/persistence/persistence_tests.rs",
        "rust-process-spawn",
        "spawn_persistence_helper",
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
    "test-state-replaced-multi-item": ExceptionSpec(
        "dux-cli/src/app/state.rs", "rust-filesystem-effect", "test:multi_delete_skips_replaced_item_and_deletes_unchanged_item"
    ),
    "test-state-changed-evidence": ExceptionSpec(
        "dux-cli/src/app/state.rs", "rust-filesystem-effect", "test:changed_artifact_marker_blocks_state_driven_delete"
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
    "test-protected-replaced-root": ExceptionSpec(
        "dux-core/src/path_validation/protected.rs", "rust-filesystem-effect", "test:replacement_scan_root_at_the_same_path_rejects_old_target_evidence"
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
    "release-checksum-sidecars-remove": ExceptionSpec(
        ".github/workflows/release.yml", "shell-remove"
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
}

EXCEPTION_PRIMITIVES = {
    "legacy-adapter-delete-directory": "remove_dir_all",
    "legacy-adapter-delete-windows-link": "remove_dir_all",
    "legacy-adapter-delete-file": "remove_file",
    "test-delete-replaced-file": "rename",
    "test-delete-replaced-directory": "rename",
    "test-delete-missing-entry": "remove_file",
    "test-delete-replaced-symlink": "rename",
    "test-delete-changed-evidence": "rename",
    "test-delete-replaced-ancestor": "rename",
    "finder-reveal": "Command::new",
    "test-persistence-helper-spawn": "Command::new",
    "test-persistence-displace-shm": "rename",
    "test-persistence-truncate-owned-database": "set_len",
    "test-state-replaced-multi-item": "rename",
    "test-state-changed-evidence": "rename",
    "cache-write-failure-temp-remove": "remove_file",
    "cache-atomic-publish": "rename",
    "cache-publish-failure-temp-remove": "remove_file",
    "test-cache-first-temp-remove": "remove_file",
    "test-cache-second-temp-remove": "remove_file",
    "test-protected-replaced-root": "rename",
    "storage-root-handle-publish": "SetFileInformationByHandle",
    "storage-root-linux-publish": "SYS_renameat2",
    "storage-root-macos-publish": "renameatx_np",
    "test-storage-root-source-swap": "rename",
    "test-storage-final-root-rename-guard": "rename",
    "test-storage-cleanup-lock-rename-guard": "rename",
    "build-xcframework-staging-remove": "rm",
    "build-xcframework-output-remove": "rm",
    "build-xcframework-publish-move": "mv",
    "build-bindings-staging-remove": "rm",
    "release-checksum-sidecars-remove": "rm",
    "lint-list-repository-sources": "subprocess.run",
    "test-lint-rejected-xcframework-output": "subprocess.run",
    "test-lint-symlinked-xcframework-parent": "subprocess.run",
    "test-lint-fixture-git-init": "subprocess.run",
    "test-lint-fixture-git-add": "subprocess.run",
}

CLIPPY_SUPPRESSION_COUNTS = {
    "dux-core/src/cleanup/legacy_cli.rs": 7,
    "dux-cli/src/app/state.rs": 3,
    "dux-core/src/cache/mod.rs": 2,
    "dux-core/src/path_validation/protected.rs": 1,
    "dux-core/src/persistence/persistence_tests.rs": 3,
    "dux-core/src/persistence/storage.rs": 1,
    "dux-core/src/persistence/storage/windows.rs": 2,
}

CLIPPY_PRODUCT_SUPPRESSION_SYMBOLS = {
    "dux-core/src/cleanup/legacy_cli.rs": {"execute_plan"},
    "dux-cli/src/app/state.rs": {"open_in_finder"},
    "dux-core/src/cache/mod.rs": {"save_cache"},
}

LEGACY_ADAPTER_ALLOWED_PATHS = {
    "dux-cli/src/app/state.rs",
    "dux-core/src/cleanup/legacy_cli.rs",
    "scripts/check_destructive_calls.py",
    "scripts/tests/test_check_destructive_calls.py",
}

LEGACY_ADAPTER_MARKERS = (
    "LegacyCliPermanentDelete",
    "legacy_cli",
)

LEGACY_ADAPTER_MODULE_DECLARATION = "pub mod legacy_cli;"

LEGACY_ADAPTER_REEXPORT_PATTERNS = (
    re.compile(
        r"\bpub\s+use\s+(?:::)?dux_core\s*(?:;|as\b|::\s*(?:\*|cleanup\b)|::\s*\{[^}]*\b(?:self|cleanup)\b)",
        re.DOTALL,
    ),
    re.compile(r"\bpub\s+extern\s+crate\s+dux_core\b"),
)


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
        raw = re.match(r"r(#+)?\"", source[index:])
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
    separators = {";", "&&", "||", "&", "|", "(", ")", "()", "{", "}"}
    for line_number, logical in logical_lines:
        try:
            tokens = _shell_tokens(logical)
        except ValueError:
            matches.extend(_regex_matches(logical, lines, SHELL_RULES))
            continue
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
                value_command = value.split(maxsplit=1)[0]
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
                    if aliases.get(target.id) != resolved:
                        aliases[target.id] = resolved
                        changed = True
                elif isinstance(resolved, (dict, list)) and containers.get(target.id) != resolved:
                    containers[target.id] = resolved
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
    legacy_adapter_source = source
    if path == "dux-core/src/cleanup/mod.rs":
        legacy_adapter_source = legacy_adapter_source.replace(
            LEGACY_ADAPTER_MODULE_DECLARATION, ""
        )
    if path not in LEGACY_ADAPTER_ALLOWED_PATHS and any(
        marker in legacy_adapter_source for marker in LEGACY_ADAPTER_MARKERS
    ):
        findings.append(
            Finding(
                path,
                1,
                "legacy-adapter-boundary",
                "legacy CLI permanent-delete adapter may not be referenced from this source",
            )
        )
    if suffix == ".rs" and path not in LEGACY_ADAPTER_ALLOWED_PATHS:
        rust_code = _strip_c_like_comments_and_literals(source)
        if any(pattern.search(rust_code) for pattern in LEGACY_ADAPTER_REEXPORT_PATTERNS):
            findings.append(
                Finding(
                    path,
                    1,
                    "legacy-adapter-boundary",
                    "dux-core cleanup modules may not be publicly re-exported",
                )
            )
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
            C_FAMILY_RULES + SWIFT_RULES,
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
