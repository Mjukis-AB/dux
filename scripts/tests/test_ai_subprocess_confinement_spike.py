import hashlib
import json
import pathlib
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
SPIKE_ROOT = REPO_ROOT / "spikes" / "ai-subprocess-confinement"
EVIDENCE_ROOT = (
    REPO_ROOT / "docs" / "testing" / "evidence" / "macos-ai-subprocess-confinement"
)


class AiSubprocessConfinementSpikeTests(unittest.TestCase):
    def test_harness_is_non_shipping_and_uses_reviewed_launch_controls(self) -> None:
        host = (SPIKE_ROOT / "host.c").read_text(encoding="utf-8")
        child = (SPIKE_ROOT / "hostile_child.c").read_text(encoding="utf-8")
        project = (REPO_ROOT / "dux-macos" / "project.yml").read_text(encoding="utf-8")
        cargo = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")

        self.assertNotIn("ai-subprocess-confinement", project)
        self.assertNotIn("ai-subprocess-confinement", cargo)
        self.assertIn('"HOME=/private/var/empty"', host)
        self.assertIn('"PATH=/usr/bin:/bin"', host)
        self.assertIn('"TMPDIR=/private/tmp"', host)
        self.assertIn("posix_spawn_file_actions_addfchdir_np", host)
        self.assertIn("POSIX_SPAWN_CLOEXEC_DEFAULT", host)
        self.assertEqual(host.count("cleanup_owned_root(&fixture)"), 5)
        self.assertIn("entry_matches", host)
        self.assertEqual(host.count("unlinkat("), 4)
        self.assertIn("reviewed_child_path(arguments[0]", host)
        self.assertIn('static const char child_name[] = "hostile-child"', host)
        self.assertEqual(host.count("posix_spawn("), 1)
        self.assertNotIn("execve(", host)
        self.assertNotIn("system(", host)
        self.assertNotIn("popen(", host)
        self.assertNotIn("/bin/sh", host)
        self.assertIn("O_NOFOLLOW", child)
        self.assertNotIn("printf", child)
        self.assertNotIn("fprintf", child)

    def test_schema_is_closed_bounded_and_marks_comparison_ineligible(self) -> None:
        schema = json.loads((SPIKE_ROOT / "evidence-v1.schema.json").read_text())
        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertFalse(schema["additionalProperties"])
        self.assertEqual(schema["properties"]["schema_version"]["const"], 1)
        self.assertEqual(schema["properties"]["probe_revision"]["const"], 1)
        for name in [
            "source",
            "platform",
            "controls",
            "direct",
            "deprecated_sandbox_exec",
            "decision",
        ]:
            self.assertFalse(schema["properties"][name]["additionalProperties"])
        self.assertFalse(
            schema["properties"]["deprecated_sandbox_exec"]["properties"]
            ["production_eligible"]["const"]
        )
        self.assertEqual(
            schema["properties"]["decision"]["properties"]["direct_local_adapter"]["enum"],
            ["no_go", "inconclusive"],
        )
        no_go_contract = schema["allOf"][0]["then"]["properties"]
        self.assertTrue(no_go_contract["controls"]["properties"]["cleanup_complete"]["const"])
        self.assertEqual(
            no_go_contract["direct"]["properties"]["result"]["const"],
            "ambient_read_succeeded",
        )
        self.assertFalse(no_go_contract["direct"]["properties"]["confined"]["const"])
        self.assertEqual(schema["$defs"]["fact"]["maxLength"], 63)

    def test_recorded_evidence_is_path_free_and_matches_the_no_go_contract(self) -> None:
        records = sorted(EVIDENCE_ROOT.glob("*.json"))
        self.assertEqual(len(records), 1)
        record = json.loads(records[0].read_text(encoding="utf-8"))
        self.assertEqual(
            set(record),
            {
                "schema_version",
                "probe_revision",
                "source",
                "platform",
                "controls",
                "direct",
                "deprecated_sandbox_exec",
                "decision",
            },
        )
        self.assertEqual(record["schema_version"], 1)
        self.assertEqual(record["probe_revision"], 1)
        self.assertEqual(
            record["source"]["schema_sha256"],
            hashlib.sha256((SPIKE_ROOT / "evidence-v1.schema.json").read_bytes()).hexdigest(),
        )
        self.assertEqual(
            record["source"]["host_source_sha256"],
            hashlib.sha256((SPIKE_ROOT / "host.c").read_bytes()).hexdigest(),
        )
        self.assertEqual(
            record["source"]["child_source_sha256"],
            hashlib.sha256((SPIKE_ROOT / "hostile_child.c").read_bytes()).hexdigest(),
        )
        self.assertRegex(record["source"]["repository_base_commit"], r"^[0-9a-f]{40}$")
        self.assertEqual(record["source"]["compiler_id"], "apple-clang")
        self.assertTrue(record["source"]["warnings_as_errors"])
        self.assertEqual(record["controls"]["sentinel_mode"], "0600")
        self.assertTrue(record["controls"]["clean_environment"])
        self.assertTrue(record["controls"]["empty_working_directory"])
        self.assertTrue(record["controls"]["nonstandard_descriptors_closed"])
        self.assertTrue(record["controls"]["known_absolute_path_disclosed"])
        self.assertFalse(record["controls"]["canary_content_emitted"])
        self.assertTrue(record["controls"]["cleanup_complete"])
        self.assertEqual(record["direct"]["result"], "ambient_read_succeeded")
        self.assertEqual(record["direct"]["child_exit_code"], 42)
        self.assertFalse(record["direct"]["confined"])
        self.assertFalse(record["deprecated_sandbox_exec"]["production_eligible"])
        self.assertEqual(record["decision"]["direct_local_adapter"], "no_go")

        encoded = json.dumps(record, sort_keys=True).lower()
        for forbidden in [
            "/private/",
            "/users/",
            "\\users\\",
            "sentinel_path",
            "username",
            "process_id",
            "environment_value",
            "dux-confinement-canary",
        ]:
            self.assertNotIn(forbidden, encoded)


if __name__ == "__main__":
    unittest.main()
