from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import stat
import sys
import unittest
from unittest import mock


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
HARNESS = REPO_ROOT / "scripts" / "qualify-icloud-v58-read-only.sh"
VALIDATOR = REPO_ROOT / "scripts" / "validate-icloud-v58-evidence.py"
SCHEMA = (
    REPO_ROOT
    / "spikes"
    / "icloud-v58-read-only-qualification"
    / "evidence-v1.schema.json"
)

SPEC = importlib.util.spec_from_file_location("dux_icloud_v58_validator", VALIDATOR)
assert SPEC is not None and SPEC.loader is not None
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


def expected(phase: str = "baseline") -> object:
    return validator.ExpectedEvidence(
        repository_commit="a" * 40,
        schema_sha256="b" * 64,
        run_started_at_unix_ns=1_800_000_000_000_000_000,
        product_version="14.7.6",
        build="23H626",
        architecture="arm64",
        account_label="acct-ABCDEFGH2345",
        fixture_label="fixture-BCDEFGH23456",
        phase=phase,
        network="online",
        sync_eligibility="eligible",
        identity_readiness="ready",
    )


def evidence(phase: str = "baseline") -> dict[str, object]:
    continuity = "baseline_recorded" if phase == "baseline" else "same_as_baseline"
    observation = {
        "stability": {
            "account": "stable",
            "provider_domain": "stable",
            "provider_item": "stable",
            "generation": "stable",
            "file_version": "stable",
        },
        "continuity": {
            "account": continuity,
            "provider_domain": continuity,
            "provider_item": continuity,
            "generation": continuity,
            "file_version": continuity,
        },
        "shared": "no",
        "sync_paused": "no",
        "sync_eligibility": "eligible",
        "identity_readiness": "ready",
    }
    return {
        "schema_version": 1,
        "protocol_contract": 58,
        "source": {
            "repository_commit": "a" * 40,
            "schema_sha256": "b" * 64,
            "workspace_clean": True,
        },
        "platform": {
            "product": "macOS",
            "product_version": "14.7.6",
            "build": "23H626",
            "architecture": "arm64",
        },
        "fixture": {
            "account_label": "acct-ABCDEFGH2345",
            "fixture_label": "fixture-BCDEFGH23456",
            "phase": phase,
            "network": "online",
        },
        "execution": {
            "run_started_at_unix_ns": 1_800_000_000_000_000_000,
            "observation_count": 3,
            "consecutive_exact_observations": True,
            "exclusive_serialization": True,
            "read_only": True,
            "production_eligible": False,
        },
        "isolation": {
            "application_database_accessed": False,
            "candidate_created": False,
            "plan_created": False,
            "journal_or_history_written": False,
            "provider_command_issued": False,
            "effect_attempted": False,
            "fixture_metadata_mutated": False,
            "fixture_content_mutated": False,
            "content_read": False,
            "account_mutated": False,
            "network_mutated": False,
        },
        "expectation": {
            "sync_eligibility": "eligible",
            "identity_readiness": "ready",
        },
        "observations": [dict(observation, sequence=index) for index in (1, 2, 3)],
        "invariants": {
            "stat_unchanged": True,
            "allocation_unchanged": True,
            "external_content_reference_confirmed": True,
            "raw_identity_emitted": False,
            "path_emitted": False,
            "filename_emitted": False,
            "content_or_hash_emitted": False,
            "error_detail_emitted": False,
            "persistent_identity_state_private": True,
        },
    }


class ICloudV58QualificationPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.harness = HARNESS.read_text(encoding="utf-8")
        self.validator_source = VALIDATOR.read_text(encoding="utf-8")
        self.schema = json.loads(SCHEMA.read_text(encoding="utf-8"))

    def test_harness_is_executable_exactly_scoped_and_default_deny(self) -> None:
        self.assertTrue(HARNESS.stat().st_mode & stat.S_IXUSR)
        self.assertIn("if (( $# != 0 )); then", self.harness)
        self.assertIn('[[ "$(uname -s)" != "Darwin" ]]', self.harness)
        self.assertIn('/usr/bin/python3 "$VALIDATOR" preflight', self.harness)
        self.assertIn("status --porcelain=v1 --untracked-files=all", self.harness)
        self.assertIn("rev-parse --verify 'HEAD^{commit}'", self.harness)
        self.assertIn("PRODUCT_MAJOR < 14", self.harness)
        self.assertIn('/usr/bin/lockf -kn "$LOCK_PATH"', self.harness)
        self.assertIn("/usr/bin/env -i", self.harness)
        self.assertIn("-scheme DuxICloudReadOnlyQualification", self.harness)
        self.assertIn(
            'LIVE_TEST="DuxICloudQualificationTests/ICloudV58ReadOnlyQualificationTests/testLiveReadOnlyQualification"',
            self.harness,
        )
        self.assertIn('-only-testing:"$LIVE_TEST"', self.harness)
        self.assertIn("-disableAutomaticPackageResolution", self.harness)
        self.assertIn("-onlyUsePackageVersionsFromResolvedFile", self.harness)
        self.assertIn(
            "qualification source changed while the test was running",
            self.harness,
        )
        self.assertEqual(
            self.harness.count("status --porcelain=v1 --untracked-files=all"),
            2,
        )
        self.assertNotIn("DUX_ICLOUD_DESTRUCTIVE_TESTS=", self.harness)
        self.assertNotIn("-parallel-testing-enabled", self.harness)
        for forbidden in (
            "evictUbiquitousItem",
            "evictItem",
            "trashItem",
            "removeItem",
            "networksetup",
            "osascript",
            "sudo ",
            "rm ",
            "rmdir ",
            "unlink ",
        ):
            self.assertNotIn(forbidden, self.harness)

    def test_preflight_checks_every_gate_before_touching_fixture_scope(self) -> None:
        self.assertLess(
            self.validator_source.index("_operator_values()"),
            self.validator_source.index('values["DUX_ICLOUD_FIXTURE_ROOT"]'),
        )
        for required in (
            '"DUX_ICLOUD_REAL_DEVICE_TESTS", "1"',
            '"DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED", "YES"',
            '"DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED", "YES"',
            '"DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED", "YES"',
            'if "DUX_ICLOUD_DESTRUCTIVE_TESTS" in os.environ',
            "fixture_link_count_refused",
            "evidence_already_exists",
            "private_directory_policy",
            "private_path_scope_refused",
            "path_not_canonical",
            '"Application Support" / "Dux"',
            '"Caches" / "Dux"',
        ):
            self.assertIn(required, self.validator_source)

        minimum_environment = {
            "DUX_ICLOUD_REAL_DEVICE_TESTS": "1",
            "DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED": "YES",
            "DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED": "YES",
            "DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED": "YES",
            "DUX_ICLOUD_DESTRUCTIVE_TESTS": "0",
        }
        with mock.patch.dict("os.environ", minimum_environment, clear=True):
            with self.assertRaisesRegex(validator.ContractError, "destructive_opt_in_present"):
                validator.validate_preflight()

    def test_schema_is_closed_bounded_and_contains_no_identity_payload_slot(self) -> None:
        self.assertEqual(self.schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertFalse(self.schema["additionalProperties"])
        self.assertEqual(self.schema["properties"]["protocol_contract"]["const"], 58)
        for section in (
            "source",
            "platform",
            "fixture",
            "execution",
            "isolation",
            "expectation",
            "invariants",
        ):
            self.assertFalse(self.schema["properties"][section]["additionalProperties"])
        self.assertFalse(self.schema["$defs"]["observation"]["additionalProperties"])
        observations = self.schema["properties"]["observations"]
        self.assertEqual(observations["minItems"], 3)
        self.assertEqual(observations["maxItems"], 3)
        self.assertFalse(observations["items"])
        self.assertEqual(
            self.schema["$defs"]["account_label"]["pattern"],
            "^acct-[A-Z2-7]{12}$",
        )
        self.assertEqual(
            self.schema["$defs"]["fixture_label"]["pattern"],
            "^fixture-[A-Z2-7]{12}$",
        )
        encoded = json.dumps(self.schema, sort_keys=True)
        for forbidden_key in (
            '"path"',
            '"filename"',
            '"identifier"',
            '"hmac"',
            '"token"',
            '"content"',
            '"error"',
            '"reviewer"',
            '"notes"',
        ):
            self.assertNotIn(forbidden_key, encoded)

    def test_qualification_code_is_confined_to_the_unhosted_test_target(self) -> None:
        project = (REPO_ROOT / "dux-macos" / "project.yml").read_text(encoding="utf-8")
        reader = (
            REPO_ROOT
            / "dux-macos"
            / "Dux"
            / "Services"
            / "ICloudLocalCopyRawFactReader.swift"
        ).read_text(encoding="utf-8")
        attributes = (REPO_ROOT / ".gitattributes").read_text(encoding="utf-8")
        targets = project.split("\ntargets:\n", 1)[1]
        app_target = targets.split("  Dux:\n", 1)[1].split("\n  DuxTests:\n", 1)[0]
        qualification_target = targets.split("\n  DuxICloudQualificationTests:\n", 1)[1]

        self.assertEqual(project.count("DUX_ICLOUD_READ_ONLY_QUALIFICATION"), 1)
        self.assertIn("DuxICloudQualificationTests:\n    type: bundle.unit-test", project)
        self.assertNotIn("TEST_HOST", qualification_target)
        self.assertNotIn("DuxICloudQualificationTests", app_target)
        self.assertGreaterEqual(
            reader.count("#if DUX_ICLOUD_READ_ONLY_QUALIFICATION"), 3
        )
        self.assertEqual(reader.count("import CryptoKit"), 1)
        for path in (
            "scripts/qualify-icloud-v58-read-only.sh text eol=lf",
            "scripts/validate-icloud-v58-evidence.py text eol=lf",
            "spikes/icloud-v58-read-only-qualification/evidence-v1.schema.json text eol=lf",
        ):
            self.assertIn(path, attributes)

    def test_validator_accepts_only_three_exact_redacted_observations(self) -> None:
        record = evidence()
        validator.validate_document(record, expected())

        mutations: list[dict[str, object]] = []
        unknown = copy.deepcopy(record)
        unknown["raw_identifier"] = "secret"
        mutations.append(unknown)
        wrong_boolean = copy.deepcopy(record)
        wrong_boolean["execution"]["observation_count"] = True
        mutations.append(wrong_boolean)
        inconsistent = copy.deepcopy(record)
        inconsistent["observations"][1]["shared"] = "unknown"
        mutations.append(inconsistent)
        leaked = copy.deepcopy(record)
        leaked["observations"][0]["continuity"]["provider_item"] = "/Users/private"
        mutations.append(leaked)
        effect = copy.deepcopy(record)
        effect["isolation"]["effect_attempted"] = True
        mutations.append(effect)
        not_private = copy.deepcopy(record)
        not_private["invariants"]["persistent_identity_state_private"] = False
        mutations.append(not_private)
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)):
                with self.assertRaises(validator.ContractError):
                    validator.validate_document(mutation, expected())

    def test_validator_ties_stability_continuity_phase_and_policy(self) -> None:
        post_restart = evidence("process_restart")
        validator.validate_document(post_restart, expected("process_restart"))

        wrong_baseline = copy.deepcopy(post_restart)
        for observation in wrong_baseline["observations"]:
            observation["continuity"]["account"] = "baseline_recorded"
        with self.assertRaisesRegex(
            validator.ContractError, "post_baseline_continuity_mismatch"
        ):
            validator.validate_document(wrong_baseline, expected("process_restart"))

        missing = evidence()
        for observation in missing["observations"]:
            observation["stability"]["provider_item"] = "unavailable"
            observation["continuity"]["provider_item"] = "unavailable"
        with self.assertRaisesRegex(
            validator.ContractError, "identity_ready_without_complete_evidence"
        ):
            validator.validate_document(missing, expected())

    def test_json_parser_rejects_duplicates_and_canonical_form_is_deterministic(self) -> None:
        with self.assertRaisesRegex(validator.ContractError, "duplicate_json_key"):
            json.loads(
                '{"schema_version":1,"schema_version":1}',
                object_pairs_hook=validator._duplicate_rejecting_object,
            )
        record = evidence()
        canonical = (
            json.dumps(record, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
            + "\n"
        ).encode("ascii")
        self.assertLess(len(canonical), validator.MAX_EVIDENCE_BYTES)
        self.assertNotIn(b"/Users/", canonical)
        self.assertNotIn(b"identifier", canonical)
        self.assertNotIn(b"hmac", canonical.lower())

    def test_live_execution_is_not_reachable_from_ordinary_policy_tests(self) -> None:
        self.assertNotIn("subprocess", self.validator_source)
        self.assertNotIn("xcodebuild", self.validator_source)
        self.assertNotIn("qualify-icloud-v58-read-only.sh", __file__)
        self.assertIn(
            'if arguments.mode == "preflight":',
            self.validator_source,
        )


if __name__ == "__main__":
    unittest.main()
