import importlib.util
import json
import pathlib
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
FINALIZER = (
    REPO_ROOT / "dux-macos/scripts/finalize-bundled-cli-metadata.py"
)
SPEC = importlib.util.spec_from_file_location("bundled_cli_metadata", FINALIZER)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BundledCliMetadataTests(unittest.TestCase):
    def source(self, **changes: object) -> bytes:
        value: dict[str, object] = {
            "record_version": 1,
            "product": "dux-cli",
            "version": "1.2.3",
            "database_schema_version": 16,
            "snapshot_format_version": 1,
        }
        value.update(changes)
        return json.dumps(value).encode("utf-8")

    def finalize(self, raw: bytes) -> bytes:
        return MODULE.finalize_metadata(
            raw,
            expected_version="1.2.3",
            sha256="ab" * 32,
        )

    def test_emits_exact_canonical_seven_key_manifest(self) -> None:
        output = self.finalize(self.source())

        self.assertEqual(
            output,
            (
                '{"record_version":1,"product":"dux-cli","version":"1.2.3",'
                '"database_schema_version":16,"snapshot_format_version":1,'
                f'"sha256":"{"ab" * 32}",'
                '"architectures":["arm64","x86_64"]}\n'
            ).encode("utf-8"),
        )

    def test_rejects_unknown_missing_duplicate_and_malformed_fields(self) -> None:
        invalid = [
            self.source(extra=True),
            json.dumps(
                {
                    "record_version": 1,
                    "product": "dux",
                    "version": "1.2.3",
                    "database_schema_version": 16,
                }
            ).encode("utf-8"),
            (
                b'{"record_version":1,"record_version":1,"product":"dux-cli",'
                b'"version":"1.2.3","database_schema_version":16,'
                b'"snapshot_format_version":1}'
            ),
            b"[]",
            b"{",
            b"\xff",
        ]

        for raw in invalid:
            with self.subTest(raw=raw):
                with self.assertRaises(MODULE.MetadataError):
                    self.finalize(raw)

    def test_rejects_wrong_identity_version_or_scalar_types(self) -> None:
        for changes in [
            {"record_version": 2},
            {"record_version": True},
            {"product": "not-dux"},
            {"version": "1.2.4"},
            {"database_schema_version": 0},
            {"database_schema_version": True},
            {"database_schema_version": 2**32},
            {"snapshot_format_version": 0},
            {"snapshot_format_version": "1"},
        ]:
            with self.subTest(changes=changes):
                with self.assertRaises(MODULE.MetadataError):
                    self.finalize(self.source(**changes))

    def test_rejects_empty_oversized_and_invalid_hash(self) -> None:
        for raw in [b"", b" " * (MODULE.MAX_METADATA_BYTES + 1)]:
            with self.subTest(size=len(raw)):
                with self.assertRaises(MODULE.MetadataError):
                    self.finalize(raw)

        with self.assertRaises(MODULE.MetadataError):
            MODULE.finalize_metadata(
                self.source(),
                expected_version="1.2.3",
                sha256="AB" * 32,
            )

    def test_final_manifest_verification_is_exact_and_can_rebind_hash(self) -> None:
        first = self.finalize(self.source())
        verified = MODULE.validate_final_metadata(
            first,
            expected_version="1.2.3",
            sha256="ab" * 32,
        )
        self.assertEqual(verified, first)

        rebound = MODULE.validate_final_metadata(
            first,
            expected_version="1.2.3",
            sha256="cd" * 32,
            rebind_hash=True,
        )
        self.assertIn(f'"sha256":"{"cd" * 32}"'.encode("utf-8"), rebound)
        with self.assertRaises(MODULE.MetadataError):
            MODULE.validate_final_metadata(
                first,
                expected_version="1.2.3",
                sha256="cd" * 32,
            )

    def test_final_manifest_rejects_shape_architecture_and_hash_types(self) -> None:
        valid = json.loads(self.finalize(self.source()))
        invalid = []
        for key in list(valid):
            missing = dict(valid)
            del missing[key]
            invalid.append(missing)
        invalid.extend(
            [
                {**valid, "extra": 1},
                {**valid, "architectures": ["x86_64", "arm64"]},
                {**valid, "architectures": ["arm64"]},
                {**valid, "architectures": "arm64,x86_64"},
                {**valid, "sha256": "AB" * 32},
                {**valid, "sha256": 1},
            ]
        )

        for value in invalid:
            with self.subTest(value=value):
                with self.assertRaises(MODULE.MetadataError):
                    MODULE.validate_final_metadata(
                        json.dumps(value).encode("utf-8"),
                        expected_version="1.2.3",
                        sha256="ab" * 32,
                    )


if __name__ == "__main__":
    unittest.main()
