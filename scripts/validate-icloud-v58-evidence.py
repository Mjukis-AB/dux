#!/usr/bin/env python3
"""Validate the redacted, read-only iCloud v58 qualification contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import pwd
import re
import stat
import sys
from dataclasses import dataclass
from typing import NoReturn


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parent.parent
SCHEMA_PATH = (
    REPOSITORY_ROOT
    / "spikes"
    / "icloud-v58-read-only-qualification"
    / "evidence-v1.schema.json"
)
MAX_EVIDENCE_BYTES = 64 * 1024

ACCOUNT_LABEL_RE = re.compile(r"acct-[A-Z2-7]{12}\Z")
FIXTURE_LABEL_RE = re.compile(r"fixture-[A-Z2-7]{12}\Z")
COMMIT_RE = re.compile(r"[0-9a-f]{40}\Z")
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
VERSION_RE = re.compile(r"[0-9]+(?:\.[0-9]+){1,3}\Z")
FACT_RE = re.compile(r"[A-Za-z0-9._-]{1,31}\Z")
OUTPUT_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,54}\.json\Z")

PHASES = frozenset(
    {
        "baseline",
        "process_restart",
        "post_reboot",
        "post_account_session",
        "metadata_refresh",
        "content_edit",
        "rename",
        "move_within_tree",
        "move_between_containers",
        "remote_download",
        "network_offline",
        "network_restored",
        "sync_paused",
        "sync_resumed",
        "sharing_enabled",
        "sharing_disabled",
        "account_changed",
    }
)
NETWORK_STATES = frozenset({"online", "offline", "restored"})
POLICY_RESULTS = frozenset({"eligible", "blocked"})
IDENTITY_RESULTS = frozenset({"ready", "blocked"})
STABILITY_VALUES = frozenset(
    {"stable", "unavailable", "changed_during_read", "unsupported"}
)
CONTINUITY_VALUES = frozenset(
    {
        "baseline_recorded",
        "same_as_baseline",
        "changed_from_baseline",
        "no_baseline",
        "unavailable",
        "changed_during_read",
        "unsupported",
    }
)
TRI_STATE_VALUES = frozenset({"yes", "no", "unknown"})
IDENTITY_FIELDS = (
    "account",
    "provider_domain",
    "provider_item",
    "generation",
    "file_version",
)

OPERATOR_ENVIRONMENT = (
    "DUX_ICLOUD_REAL_DEVICE_TESTS",
    "DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED",
    "DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED",
    "DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED",
    "DUX_ICLOUD_FIXTURE_ROOT",
    "DUX_ICLOUD_FIXTURE_PATH",
    "DUX_ICLOUD_ACCOUNT_LABEL",
    "DUX_ICLOUD_FIXTURE_LABEL",
    "DUX_ICLOUD_PHASE",
    "DUX_ICLOUD_NETWORK",
    "DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY",
    "DUX_ICLOUD_EXPECTED_IDENTITY_READINESS",
    "DUX_ICLOUD_PRIVATE_STATE_DIRECTORY",
    "DUX_ICLOUD_EVIDENCE_OUTPUT",
)
INTERNAL_ENVIRONMENT = (
    "DUX_ICLOUD_SOURCE_COMMIT",
    "DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256",
    "DUX_ICLOUD_RUN_STARTED_AT_UNIX_NS",
    "DUX_ICLOUD_EXPECTED_OS_VERSION",
    "DUX_ICLOUD_EXPECTED_OS_BUILD",
    "DUX_ICLOUD_EXPECTED_ARCHITECTURE",
    "DUX_ICLOUD_EXCLUSIVE_SERIALIZATION",
)


class ContractError(Exception):
    """A fixed, non-sensitive refusal code."""

    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


@dataclass(frozen=True)
class ExpectedEvidence:
    repository_commit: str
    schema_sha256: str
    run_started_at_unix_ns: int
    product_version: str
    build: str
    architecture: str
    account_label: str
    fixture_label: str
    phase: str
    network: str
    sync_eligibility: str
    identity_readiness: str


def _fail(code: str) -> NoReturn:
    raise ContractError(code)


def _required_environment(name: str) -> str:
    value = os.environ.get(name)
    if value is None or value == "" or "\x00" in value or "\n" in value or "\r" in value:
        _fail("required_environment")
    return value


def _exact_environment(name: str, expected: str) -> None:
    if _required_environment(name) != expected:
        _fail("confirmation_refused")


def _enum_environment(name: str, allowed: frozenset[str]) -> str:
    value = _required_environment(name)
    if value not in allowed:
        _fail("environment_enum")
    return value


def _is_within(path: pathlib.Path, parent: pathlib.Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def _canonical_existing_path(raw: str, *, directory: bool) -> pathlib.Path:
    candidate = pathlib.Path(raw)
    if not candidate.is_absolute() or raw != os.path.realpath(raw):
        _fail("path_not_canonical")
    try:
        metadata = os.lstat(candidate)
    except OSError:
        _fail("path_unavailable")
    expected_kind = stat.S_ISDIR if directory else stat.S_ISREG
    if stat.S_ISLNK(metadata.st_mode) or not expected_kind(metadata.st_mode):
        _fail("path_wrong_kind")
    return candidate


def _private_owned_directory(raw: str) -> pathlib.Path:
    directory = _canonical_existing_path(raw, directory=True)
    metadata = os.stat(directory, follow_symlinks=False)
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        _fail("private_directory_policy")
    return directory


def _validate_optional_private_entry(
    parent: pathlib.Path,
    name: str,
    *,
    directory: bool,
    expected_mode: int,
) -> None:
    candidate = parent / name
    try:
        metadata = os.lstat(candidate)
    except FileNotFoundError:
        return
    except OSError:
        _fail("private_control_entry")
    expected_kind = stat.S_ISDIR if directory else stat.S_ISREG
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not expected_kind(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or metadata.st_nlink != 1
        or stat.S_IMODE(metadata.st_mode) != expected_mode
    ):
        _fail("private_control_entry")


def _is_icloud_path(path: pathlib.Path) -> bool:
    parts = path.parts
    return "Mobile Documents" in parts or "CloudStorage" in parts


def _is_dux_app_storage(path: pathlib.Path) -> bool:
    home = pathlib.Path(pwd.getpwuid(os.geteuid()).pw_dir)
    protected = (
        home / "Library" / "Application Support" / "Dux",
        home / "Library" / "Application Support" / "DUX",
        home / "Library" / "Caches" / "Dux",
        home / "Library" / "Caches" / "com.dux",
        home / "Library" / "Caches" / "DUX",
        home / "Library" / "Containers",
    )
    return any(_is_within(path, item) for item in protected)


def _operator_values() -> dict[str, str]:
    _exact_environment("DUX_ICLOUD_REAL_DEVICE_TESTS", "1")
    if "DUX_ICLOUD_DESTRUCTIVE_TESTS" in os.environ:
        _fail("destructive_opt_in_present")
    _exact_environment("DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED", "YES")
    _exact_environment("DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED", "YES")
    _exact_environment("DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED", "YES")

    values = {name: _required_environment(name) for name in OPERATOR_ENVIRONMENT}
    if ACCOUNT_LABEL_RE.fullmatch(values["DUX_ICLOUD_ACCOUNT_LABEL"]) is None:
        _fail("account_label_not_opaque")
    if FIXTURE_LABEL_RE.fullmatch(values["DUX_ICLOUD_FIXTURE_LABEL"]) is None:
        _fail("fixture_label_not_opaque")
    if values["DUX_ICLOUD_PHASE"] not in PHASES:
        _fail("phase_not_supported")
    if values["DUX_ICLOUD_NETWORK"] not in NETWORK_STATES:
        _fail("network_not_supported")
    if values["DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY"] not in POLICY_RESULTS:
        _fail("sync_expectation_not_supported")
    if values["DUX_ICLOUD_EXPECTED_IDENTITY_READINESS"] not in IDENTITY_RESULTS:
        _fail("identity_expectation_not_supported")
    if (
        values["DUX_ICLOUD_PHASE"] == "network_offline"
        and values["DUX_ICLOUD_NETWORK"] != "offline"
    ) or (
        values["DUX_ICLOUD_PHASE"] == "network_restored"
        and values["DUX_ICLOUD_NETWORK"] != "restored"
    ):
        _fail("phase_network_mismatch")
    return values


def validate_preflight() -> None:
    values = _operator_values()
    for name in INTERNAL_ENVIRONMENT:
        if name in os.environ:
            _fail("reserved_environment_present")

    fixture_root = _canonical_existing_path(
        values["DUX_ICLOUD_FIXTURE_ROOT"], directory=True
    )
    fixture_path = _canonical_existing_path(
        values["DUX_ICLOUD_FIXTURE_PATH"], directory=False
    )
    if not _is_icloud_path(fixture_root) or not _is_within(fixture_path, fixture_root):
        _fail("fixture_scope_refused")
    if fixture_path.parent == fixture_root and fixture_path == fixture_root:
        _fail("fixture_scope_refused")

    fixture_metadata = os.stat(fixture_path, follow_symlinks=False)
    if fixture_metadata.st_nlink != 1:
        _fail("fixture_link_count_refused")

    state_directory = _private_owned_directory(
        values["DUX_ICLOUD_PRIVATE_STATE_DIRECTORY"]
    )
    _validate_optional_private_entry(
        state_directory,
        "qualification-v1.lock",
        directory=False,
        expected_mode=0o600,
    )
    _validate_optional_private_entry(
        state_directory,
        "derived-data-v1",
        directory=True,
        expected_mode=0o700,
    )
    output = pathlib.Path(values["DUX_ICLOUD_EVIDENCE_OUTPUT"])
    if (
        not output.is_absolute()
        or OUTPUT_NAME_RE.fullmatch(output.name) is None
        or str(output.parent) != os.path.realpath(output.parent)
    ):
        _fail("evidence_path_refused")
    output_parent = _private_owned_directory(str(output.parent))
    try:
        os.lstat(output)
    except FileNotFoundError:
        pass
    except OSError:
        _fail("evidence_path_refused")
    else:
        _fail("evidence_already_exists")

    for private_path in (state_directory, output_parent, output):
        if (
            _is_within(private_path, fixture_root)
            or _is_icloud_path(private_path)
            or _is_dux_app_storage(private_path)
            or _is_within(private_path, REPOSITORY_ROOT)
        ):
            _fail("private_path_scope_refused")
    if _is_within(fixture_root, state_directory) or _is_within(
        fixture_root, output_parent
    ):
        _fail("private_path_scope_refused")


def _exact_object(value: object, keys: set[str], code: str) -> dict[str, object]:
    if type(value) is not dict or set(value) != keys:
        _fail(code)
    return value


def _exact_array(value: object, length: int, code: str) -> list[object]:
    if type(value) is not list or len(value) != length:
        _fail(code)
    return value


def _exact_integer(value: object, code: str) -> int:
    if type(value) is not int:
        _fail(code)
    return value


def _exact_string(value: object, code: str) -> str:
    if type(value) is not str or not value.isascii():
        _fail(code)
    return value


def _const(value: object, expected: object, code: str) -> None:
    if type(value) is not type(expected) or value != expected:
        _fail(code)


def _pattern(value: object, pattern: re.Pattern[str], code: str) -> str:
    text = _exact_string(value, code)
    if pattern.fullmatch(text) is None:
        _fail(code)
    return text


def _enum(value: object, allowed: frozenset[str], code: str) -> str:
    text = _exact_string(value, code)
    if text not in allowed:
        _fail(code)
    return text


def _validate_identity_set(
    value: object, allowed: frozenset[str], code: str
) -> dict[str, object]:
    result = _exact_object(value, set(IDENTITY_FIELDS), code)
    for field in IDENTITY_FIELDS:
        _enum(result[field], allowed, code)
    return result


def validate_document(document: object, expected: ExpectedEvidence) -> None:
    top = _exact_object(
        document,
        {
            "schema_version",
            "protocol_contract",
            "source",
            "platform",
            "fixture",
            "execution",
            "isolation",
            "expectation",
            "observations",
            "invariants",
        },
        "top_level_shape",
    )
    _const(top["schema_version"], 1, "schema_version")
    _const(top["protocol_contract"], 58, "protocol_contract")

    source = _exact_object(
        top["source"],
        {"repository_commit", "schema_sha256", "workspace_clean"},
        "source_shape",
    )
    _pattern(source["repository_commit"], COMMIT_RE, "repository_commit")
    _pattern(source["schema_sha256"], SHA256_RE, "schema_sha256")
    _const(source["repository_commit"], expected.repository_commit, "source_mismatch")
    _const(source["schema_sha256"], expected.schema_sha256, "schema_mismatch")
    _const(source["workspace_clean"], True, "workspace_not_clean")

    platform = _exact_object(
        top["platform"],
        {"product", "product_version", "build", "architecture"},
        "platform_shape",
    )
    _const(platform["product"], "macOS", "platform_product")
    _pattern(platform["product_version"], VERSION_RE, "platform_version")
    _pattern(platform["build"], FACT_RE, "platform_build")
    _enum(platform["architecture"], frozenset({"arm64", "x86_64"}), "architecture")
    _const(platform["product_version"], expected.product_version, "platform_mismatch")
    _const(platform["build"], expected.build, "platform_mismatch")
    _const(platform["architecture"], expected.architecture, "platform_mismatch")
    if int(expected.product_version.split(".", 1)[0]) < 14:
        _fail("unsupported_macos")

    fixture = _exact_object(
        top["fixture"],
        {"account_label", "fixture_label", "phase", "network"},
        "fixture_shape",
    )
    _pattern(fixture["account_label"], ACCOUNT_LABEL_RE, "account_label")
    _pattern(fixture["fixture_label"], FIXTURE_LABEL_RE, "fixture_label")
    _enum(fixture["phase"], PHASES, "phase")
    _enum(fixture["network"], NETWORK_STATES, "network")
    _const(fixture["account_label"], expected.account_label, "fixture_mismatch")
    _const(fixture["fixture_label"], expected.fixture_label, "fixture_mismatch")
    _const(fixture["phase"], expected.phase, "fixture_mismatch")
    _const(fixture["network"], expected.network, "fixture_mismatch")

    execution = _exact_object(
        top["execution"],
        {
            "run_started_at_unix_ns",
            "observation_count",
            "consecutive_exact_observations",
            "exclusive_serialization",
            "read_only",
            "production_eligible",
        },
        "execution_shape",
    )
    started = _exact_integer(execution["run_started_at_unix_ns"], "run_started")
    if started <= 0 or started != expected.run_started_at_unix_ns:
        _fail("run_started_mismatch")
    for key, value in {
        "observation_count": 3,
        "consecutive_exact_observations": True,
        "exclusive_serialization": True,
        "read_only": True,
        "production_eligible": False,
    }.items():
        _const(execution[key], value, "execution_contract")

    isolation = _exact_object(
        top["isolation"],
        {
            "application_database_accessed",
            "candidate_created",
            "plan_created",
            "journal_or_history_written",
            "provider_command_issued",
            "effect_attempted",
            "fixture_metadata_mutated",
            "fixture_content_mutated",
            "content_read",
            "account_mutated",
            "network_mutated",
        },
        "isolation_shape",
    )
    for value in isolation.values():
        _const(value, False, "isolation_contract")

    expectation = _exact_object(
        top["expectation"],
        {"sync_eligibility", "identity_readiness"},
        "expectation_shape",
    )
    _enum(expectation["sync_eligibility"], POLICY_RESULTS, "sync_expectation")
    _enum(expectation["identity_readiness"], IDENTITY_RESULTS, "identity_expectation")
    _const(
        expectation["sync_eligibility"], expected.sync_eligibility, "expectation_mismatch"
    )
    _const(
        expectation["identity_readiness"],
        expected.identity_readiness,
        "expectation_mismatch",
    )

    observations = _exact_array(top["observations"], 3, "observation_count")
    normalized: list[dict[str, object]] = []
    for sequence, value in enumerate(observations, start=1):
        observation = _exact_object(
            value,
            {
                "sequence",
                "stability",
                "continuity",
                "shared",
                "sync_paused",
                "sync_eligibility",
                "identity_readiness",
            },
            "observation_shape",
        )
        _const(observation["sequence"], sequence, "observation_sequence")
        stability = _validate_identity_set(
            observation["stability"], STABILITY_VALUES, "stability_shape"
        )
        continuity = _validate_identity_set(
            observation["continuity"], CONTINUITY_VALUES, "continuity_shape"
        )
        shared = _enum(observation["shared"], TRI_STATE_VALUES, "shared")
        sync_paused = _enum(
            observation["sync_paused"], TRI_STATE_VALUES, "sync_paused"
        )
        sync_eligibility = _enum(
            observation["sync_eligibility"], POLICY_RESULTS, "sync_eligibility"
        )
        identity_readiness = _enum(
            observation["identity_readiness"], IDENTITY_RESULTS, "identity_readiness"
        )
        _const(sync_eligibility, expected.sync_eligibility, "observed_policy_mismatch")
        _const(identity_readiness, expected.identity_readiness, "observed_policy_mismatch")
        if identity_readiness == "ready":
            if (
                sync_eligibility != "eligible"
                or shared != "no"
                or sync_paused != "no"
                or any(stability[field] != "stable" for field in IDENTITY_FIELDS)
                or any(
                    continuity[field]
                    not in {"baseline_recorded", "same_as_baseline"}
                    for field in IDENTITY_FIELDS
                )
            ):
                _fail("identity_ready_without_complete_evidence")
        for field in IDENTITY_FIELDS:
            state = stability[field]
            continuity_state = continuity[field]
            if state in {"unavailable", "changed_during_read", "unsupported"}:
                if continuity_state != state:
                    _fail("stability_continuity_mismatch")
            elif expected.phase == "baseline":
                if continuity_state != "baseline_recorded":
                    _fail("baseline_continuity_mismatch")
            elif continuity_state == "baseline_recorded":
                _fail("post_baseline_continuity_mismatch")
        normalized.append({key: item for key, item in observation.items() if key != "sequence"})
    if normalized[1:] != normalized[:1] * 2:
        _fail("observations_not_exact")
    if expected.phase == "account_changed" and expected.identity_readiness != "blocked":
        _fail("account_change_must_block")

    invariants = _exact_object(
        top["invariants"],
        {
            "stat_unchanged",
            "allocation_unchanged",
            "external_content_reference_confirmed",
            "raw_identity_emitted",
            "path_emitted",
            "filename_emitted",
            "content_or_hash_emitted",
            "error_detail_emitted",
            "persistent_identity_state_private",
        },
        "invariant_shape",
    )
    for key in (
        "stat_unchanged",
        "allocation_unchanged",
        "external_content_reference_confirmed",
        "persistent_identity_state_private",
    ):
        _const(invariants[key], True, "required_invariant_missing")
    for key in (
        "raw_identity_emitted",
        "path_emitted",
        "filename_emitted",
        "content_or_hash_emitted",
        "error_detail_emitted",
    ):
        _const(invariants[key], False, "redaction_invariant_failed")


def _duplicate_rejecting_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            _fail("duplicate_json_key")
        result[key] = value
    return result


def _reject_float(_: str) -> NoReturn:
    _fail("non_integer_json_number")


def _expected_from_environment() -> ExpectedEvidence:
    _exact_environment("DUX_ICLOUD_EXCLUSIVE_SERIALIZATION", "1")
    commit = _required_environment("DUX_ICLOUD_SOURCE_COMMIT")
    schema_hash = _required_environment("DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256")
    started_text = _required_environment("DUX_ICLOUD_RUN_STARTED_AT_UNIX_NS")
    version = _required_environment("DUX_ICLOUD_EXPECTED_OS_VERSION")
    build = _required_environment("DUX_ICLOUD_EXPECTED_OS_BUILD")
    architecture = _required_environment("DUX_ICLOUD_EXPECTED_ARCHITECTURE")
    if COMMIT_RE.fullmatch(commit) is None or SHA256_RE.fullmatch(schema_hash) is None:
        _fail("internal_source_identity")
    if VERSION_RE.fullmatch(version) is None or FACT_RE.fullmatch(build) is None:
        _fail("internal_platform_identity")
    if architecture not in {"arm64", "x86_64"}:
        _fail("internal_platform_identity")
    try:
        started = int(started_text, 10)
    except ValueError:
        _fail("internal_run_identity")
    if str(started) != started_text or started <= 0:
        _fail("internal_run_identity")
    values = _operator_values()
    return ExpectedEvidence(
        repository_commit=commit,
        schema_sha256=schema_hash,
        run_started_at_unix_ns=started,
        product_version=version,
        build=build,
        architecture=architecture,
        account_label=values["DUX_ICLOUD_ACCOUNT_LABEL"],
        fixture_label=values["DUX_ICLOUD_FIXTURE_LABEL"],
        phase=values["DUX_ICLOUD_PHASE"],
        network=values["DUX_ICLOUD_NETWORK"],
        sync_eligibility=values["DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY"],
        identity_readiness=values["DUX_ICLOUD_EXPECTED_IDENTITY_READINESS"],
    )


def validate_evidence_file() -> None:
    expected = _expected_from_environment()
    output = pathlib.Path(_required_environment("DUX_ICLOUD_EVIDENCE_OUTPUT"))
    if not output.is_absolute() or str(output.parent) != os.path.realpath(output.parent):
        _fail("evidence_path_refused")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(output, flags)
    except OSError:
        _fail("evidence_unavailable")
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_nlink != 1
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_size <= 0
            or metadata.st_size > MAX_EVIDENCE_BYTES
            or metadata.st_mtime_ns < expected.run_started_at_unix_ns
        ):
            _fail("evidence_file_policy")
        raw = os.read(descriptor, metadata.st_size + 1)
    finally:
        os.close(descriptor)
    if len(raw) != metadata.st_size or len(raw) > MAX_EVIDENCE_BYTES:
        _fail("evidence_size_changed")
    try:
        document = json.loads(
            raw,
            object_pairs_hook=_duplicate_rejecting_object,
            parse_float=_reject_float,
            parse_constant=_reject_float,
        )
    except ContractError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError):
        _fail("evidence_json_refused")
    validate_document(document, expected)
    canonical = (
        json.dumps(document, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
        + "\n"
    ).encode("ascii")
    if raw != canonical:
        _fail("evidence_not_canonical")


def _schema_sha256() -> str:
    try:
        return hashlib.sha256(SCHEMA_PATH.read_bytes()).hexdigest()
    except OSError:
        _fail("schema_unavailable")


def main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("mode", choices=("preflight", "evidence"))
    arguments = parser.parse_args()
    try:
        if arguments.mode == "preflight":
            validate_preflight()
        else:
            if _required_environment("DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256") != _schema_sha256():
                _fail("schema_mismatch")
            validate_evidence_file()
    except ContractError as error:
        print(
            f"error: iCloud v58 qualification refused [{error.code}]",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
