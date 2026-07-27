#!/usr/bin/env python3
"""Validate the repository-owned RSI Tech public project manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import sys
from pathlib import Path, PurePosixPath
from typing import Any
from urllib.parse import urlparse


TOP_LEVEL_FIELDS = {
    "schemaVersion",
    "identity",
    "positioning",
    "lifecycle",
    "platforms",
    "languages",
    "links",
    "media",
    "openSource",
    "capabilities",
    "limitations",
    "safetyBoundaries",
}
OBJECT_FIELDS = {
    "identity": {"slug", "name", "repository"},
    "positioning": {"oneLine", "description", "category"},
    "lifecycle": {"stage", "version"},
    "links": {"repository", "documentation", "release", "download", "appStore"},
    "media": {"cardImage", "screenshots"},
    "openSource": {
        "license",
        "readme",
        "contributing",
        "security",
        "support",
        "codeOfConduct",
    },
}
PATH_FIELDS = (
    ("media.cardImage", ("media", "cardImage")),
    ("openSource.readme", ("openSource", "readme")),
    ("openSource.contributing", ("openSource", "contributing")),
    ("openSource.security", ("openSource", "security")),
    ("openSource.support", ("openSource", "support")),
    ("openSource.codeOfConduct", ("openSource", "codeOfConduct")),
)
SLUG = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


class ManifestError(ValueError):
    """The public manifest violates its repository contract."""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--repo-root", required=True, type=Path)
    return parser.parse_args()


def require_object(
    parent: dict[str, Any], field: str, expected_fields: set[str]
) -> dict[str, Any]:
    value = parent.get(field)
    if not isinstance(value, dict):
        raise ManifestError(f"{field} must be an object")
    actual_fields = set(value)
    if actual_fields != expected_fields:
        missing = sorted(expected_fields - actual_fields)
        extra = sorted(actual_fields - expected_fields)
        raise ManifestError(f"{field} fields mismatch: missing={missing}, extra={extra}")
    return value


def require_text(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ManifestError(f"{field} must be non-empty text")
    return value


def require_text_array(value: Any, field: str, *, nonempty: bool) -> list[str]:
    if not isinstance(value, list) or (nonempty and not value):
        raise ManifestError(f"{field} must be a non-empty array")
    if any(not isinstance(item, str) or not item.strip() for item in value):
        raise ManifestError(f"{field} must contain non-empty text")
    if len(value) != len(set(value)):
        raise ManifestError(f"{field} must not contain duplicates")
    return value


def validate_https(value: Any, field: str) -> None:
    if value is None:
        return
    if not isinstance(value, str):
        raise ManifestError(f"{field} must be HTTPS or null")
    parsed = urlparse(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
    ):
        raise ManifestError(f"{field} must be credential-free HTTPS or null")


def validate_path(repo_root: Path, value: Any, field: str) -> None:
    if not isinstance(value, str) or not value:
        raise ManifestError(f"{field} must be a repository-relative path")
    relative = PurePosixPath(value)
    if relative.is_absolute() or ".." in relative.parts:
        raise ManifestError(f"{field} must remain inside the repository")

    candidate = repo_root
    for index, part in enumerate(relative.parts):
        candidate = candidate / part
        try:
            mode = candidate.lstat().st_mode
        except OSError as error:
            raise ManifestError(f"{field} does not exist: {value}") from error
        if stat.S_ISLNK(mode):
            raise ManifestError(f"{field} must not contain symlinks")
        is_final = index == len(relative.parts) - 1
        if (not is_final and not stat.S_ISDIR(mode)) or (
            is_final and not stat.S_ISREG(mode)
        ):
            raise ManifestError(f"{field} does not name a regular file: {value}")

    try:
        candidate.resolve(strict=True).relative_to(repo_root)
    except (OSError, RuntimeError, ValueError) as error:
        raise ManifestError(f"{field} must remain inside the repository") from error


def validate_manifest(data: Any, repo_root: Path) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise ManifestError("manifest must be an object")
    actual_fields = set(data)
    if actual_fields != TOP_LEVEL_FIELDS:
        missing = sorted(TOP_LEVEL_FIELDS - actual_fields)
        extra = sorted(actual_fields - TOP_LEVEL_FIELDS)
        raise ManifestError(f"top-level fields mismatch: missing={missing}, extra={extra}")
    if data["schemaVersion"] != 1:
        raise ManifestError("schemaVersion must equal 1")

    objects = {
        name: require_object(data, name, fields)
        for name, fields in OBJECT_FIELDS.items()
    }
    identity = objects["identity"]
    slug = require_text(identity["slug"], "identity.slug")
    if not SLUG.fullmatch(slug):
        raise ManifestError("identity.slug is invalid")
    require_text(identity["name"], "identity.name")
    repository = require_text(identity["repository"], "identity.repository")
    if not repository.startswith("rsitech-ai/") or repository.count("/") != 1:
        raise ManifestError("identity.repository must be owned by rsitech-ai")

    positioning = objects["positioning"]
    for field in ("oneLine", "description", "category"):
        require_text(positioning[field], f"positioning.{field}")

    lifecycle = objects["lifecycle"]
    if lifecycle["stage"] not in {"public-preview", "released"}:
        raise ManifestError("lifecycle.stage must be public-preview or released")
    if lifecycle["version"] is not None:
        require_text(lifecycle["version"], "lifecycle.version")

    require_text_array(data["platforms"], "platforms", nonempty=True)
    require_text_array(data["languages"], "languages", nonempty=True)
    require_text_array(data["limitations"], "limitations", nonempty=False)
    require_text_array(data["safetyBoundaries"], "safetyBoundaries", nonempty=False)

    links = objects["links"]
    if links["repository"] != f"https://github.com/{repository}":
        raise ManifestError("links.repository does not match identity.repository")
    for field, value in links.items():
        validate_https(value, f"links.{field}")

    open_source = objects["openSource"]
    require_text(open_source["license"], "openSource.license")
    media = objects["media"]
    screenshots = media["screenshots"]
    if not isinstance(screenshots, list):
        raise ManifestError("media.screenshots must be an array")

    for field, path in PATH_FIELDS:
        first, second = path
        validate_path(repo_root, data[first][second], field)
    for index, screenshot in enumerate(screenshots):
        if not isinstance(screenshot, dict) or set(screenshot) != {"path", "alt"}:
            raise ManifestError(f"media.screenshots.{index} has invalid fields")
        require_text(screenshot["alt"], f"media.screenshots.{index}.alt")
        validate_path(
            repo_root, screenshot["path"], f"media.screenshots.{index}.path"
        )

    capabilities = data["capabilities"]
    if not isinstance(capabilities, list):
        raise ManifestError("capabilities must be an array")
    for index, capability in enumerate(capabilities):
        if not isinstance(capability, dict) or set(capability) != {"claim", "evidence"}:
            raise ManifestError(f"capabilities.{index} has invalid fields")
        require_text(capability["claim"], f"capabilities.{index}.claim")
        evidence = require_text_array(
            capability["evidence"], f"capabilities.{index}.evidence", nonempty=True
        )
        for evidence_index, value in enumerate(evidence):
            validate_path(
                repo_root,
                value,
                f"capabilities.{index}.evidence.{evidence_index}",
            )
    return data


def main() -> int:
    arguments = parse_args()
    try:
        repo_root = arguments.repo_root.resolve(strict=True)
        if not repo_root.is_dir() or repo_root.is_symlink():
            raise ManifestError("repository root must be a real directory")
        with arguments.manifest.open("r", encoding="utf-8") as handle:
            data = json.load(handle)
        manifest = validate_manifest(data, repo_root)
    except (OSError, json.JSONDecodeError, ManifestError) as error:
        print(f"project manifest invalid: {error}", file=sys.stderr)
        return 2

    encoded = json.dumps(
        manifest, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    result = {
        "manifest_hash": hashlib.sha256(encoded).hexdigest(),
        "readiness": (
            "release-ready"
            if manifest["lifecycle"]["stage"] == "released"
            else "preview-ready"
        ),
    }
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
