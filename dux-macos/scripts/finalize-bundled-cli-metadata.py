#!/usr/bin/env python3
"""Validate trusted CLI metadata and emit the signed-bundle manifest."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
from typing import Any


MAX_METADATA_BYTES = 4_096
MAX_U32 = 2**32 - 1
SOURCE_KEYS = {
    "record_version",
    "product",
    "version",
    "database_schema_version",
    "snapshot_format_version",
}
FINAL_KEYS = SOURCE_KEYS | {"sha256", "architectures"}
SHA256_PATTERN = re.compile(r"[0-9a-f]{64}")


class MetadataError(ValueError):
    """The trusted metadata output did not match its closed contract."""


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise MetadataError(f"duplicate metadata key: {key}")
        value[key] = item
    return value


def _require_u32(value: Any, key: str) -> int:
    if type(value) is not int or not 1 <= value <= MAX_U32:
        raise MetadataError(f"{key} must be a positive unsigned 32-bit integer")
    return value


def _decode_object(raw: bytes) -> dict[str, Any]:
    if not raw or len(raw) > MAX_METADATA_BYTES:
        raise MetadataError("metadata output must contain 1 to 4096 bytes")
    try:
        decoded = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise MetadataError("metadata output must be UTF-8") from error
    try:
        value = json.loads(decoded, object_pairs_hook=_reject_duplicate_keys)
    except (json.JSONDecodeError, MetadataError) as error:
        raise MetadataError(f"metadata output is not strict JSON: {error}") from error
    if type(value) is not dict:
        raise MetadataError("metadata output must be one JSON object")
    return value


def _validate_common(source: dict[str, Any], expected_version: str) -> tuple[int, int]:
    if type(source["record_version"]) is not int or source["record_version"] != 1:
        raise MetadataError("record_version must equal 1")
    if source["product"] != "dux-cli":
        raise MetadataError("product must equal dux-cli")
    if type(source["version"]) is not str or source["version"] != expected_version:
        raise MetadataError("version does not match the dux-cli package")
    database_schema_version = _require_u32(
        source["database_schema_version"],
        "database_schema_version",
    )
    snapshot_format_version = _require_u32(
        source["snapshot_format_version"],
        "snapshot_format_version",
    )
    return database_schema_version, snapshot_format_version


def _encode_manifest(
    *,
    expected_version: str,
    database_schema_version: int,
    snapshot_format_version: int,
    sha256: str,
) -> bytes:
    manifest = {
        "record_version": 1,
        "product": "dux-cli",
        "version": expected_version,
        "database_schema_version": database_schema_version,
        "snapshot_format_version": snapshot_format_version,
        "sha256": sha256,
        "architectures": ["arm64", "x86_64"],
    }
    return (
        json.dumps(manifest, ensure_ascii=True, separators=(",", ":")) + "\n"
    ).encode("utf-8")


def finalize_metadata(
    raw: bytes,
    *,
    expected_version: str,
    sha256: str,
) -> bytes:
    source = _decode_object(raw)
    if set(source) != SOURCE_KEYS:
        raise MetadataError("metadata output has missing or unknown keys")
    database_schema_version, snapshot_format_version = _validate_common(
        source,
        expected_version,
    )
    if SHA256_PATTERN.fullmatch(sha256) is None:
        raise MetadataError("sha256 must be 64 lowercase hexadecimal characters")
    return _encode_manifest(
        expected_version=expected_version,
        database_schema_version=database_schema_version,
        snapshot_format_version=snapshot_format_version,
        sha256=sha256,
    )


def validate_final_metadata(
    raw: bytes,
    *,
    expected_version: str,
    sha256: str,
    rebind_hash: bool = False,
) -> bytes:
    source = _decode_object(raw)
    if set(source) != FINAL_KEYS:
        raise MetadataError("final manifest has missing or unknown keys")
    database_schema_version, snapshot_format_version = _validate_common(
        source,
        expected_version,
    )
    if type(source["sha256"]) is not str or SHA256_PATTERN.fullmatch(source["sha256"]) is None:
        raise MetadataError("manifest sha256 must be 64 lowercase hexadecimal characters")
    if source["architectures"] != ["arm64", "x86_64"]:
        raise MetadataError("architectures must equal [arm64, x86_64] in canonical order")
    if SHA256_PATTERN.fullmatch(sha256) is None:
        raise MetadataError("sha256 must be 64 lowercase hexadecimal characters")
    if not rebind_hash and source["sha256"] != sha256:
        raise MetadataError("manifest sha256 does not match the bundled CLI")
    return _encode_manifest(
        expected_version=expected_version,
        database_schema_version=database_schema_version,
        snapshot_format_version=snapshot_format_version,
        sha256=sha256,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=pathlib.Path)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument(
        "--mode",
        choices=("create", "verify", "rebind"),
        default="create",
    )
    arguments = parser.parse_args()
    try:
        raw = arguments.input.read_bytes()
        if arguments.mode == "create":
            output = finalize_metadata(
                raw,
                expected_version=arguments.expected_version,
                sha256=arguments.sha256,
            )
        else:
            output = validate_final_metadata(
                raw,
                expected_version=arguments.expected_version,
                sha256=arguments.sha256,
                rebind_hash=arguments.mode == "rebind",
            )
    except (OSError, MetadataError) as error:
        parser.error(str(error))
    print(output.decode("utf-8"), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
