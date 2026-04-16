#!/usr/bin/env python3
"""Helpers for deterministic Nova CLI script wrappers."""

from __future__ import annotations

import csv
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterable


JsonDict = dict[str, Any]


def default_nova_bin() -> str:
    """Return the Nova binary path for helper scripts."""

    if env_bin := os.environ.get("DBT_NOVA_BIN"):
        return env_bin
    if path_bin := shutil.which("dbt-nova"):
        return path_bin
    return "./target/debug/dbt-nova"


def chunked(items: list[str], size: int) -> Iterable[list[str]]:
    """Yield fixed-size chunks."""

    for index in range(0, len(items), size):
        yield items[index : index + size]


def manifest_identity(manifest_path: Path) -> JsonDict:
    """Return a deterministic manifest identity summary."""

    digest = hashlib.sha256()
    with manifest_path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return {
        "manifest_path": str(manifest_path),
        "sha256": digest.hexdigest(),
    }


def run_nova_tool(
    nova_bin: str | Path,
    manifest_path: str | Path,
    tool_name: str,
    params: JsonDict,
) -> JsonDict:
    """Call a Nova CLI tool and return the parsed payload."""

    command = [
        str(nova_bin),
        "tool",
        "call",
        tool_name,
        "--manifest-path",
        str(manifest_path),
        "--params-json",
        json.dumps(params, sort_keys=True, separators=(",", ":")),
        "--json",
    ]
    result = subprocess.run(
        command,
        capture_output=True,
        check=False,
        text=True,
    )
    if result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip() or "unknown nova cli error"
        raise RuntimeError(f"{tool_name} failed with exit code {result.returncode}: {message}")
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{tool_name} returned invalid JSON: {exc}") from exc
    if payload.get("status") != "success" or payload.get("error") is not None:
        raise RuntimeError(
            f"{tool_name} returned an error: {json.dumps(payload.get('error'), sort_keys=True)}"
        )
    data = payload.get("data")
    if not isinstance(data, dict):
        raise RuntimeError(f"{tool_name} returned an unexpected payload shape")
    return {
        "command": payload.get("command"),
        "meta": payload.get("meta", {}),
        "result": data,
    }


def paginated_tool_rows(
    nova_bin: str | Path,
    manifest_path: str | Path,
    tool_name: str,
    params: JsonDict,
    *,
    page_size: int = 200,
) -> list[JsonDict]:
    """Collect deterministic paginated rows from a Nova CLI tool."""

    rows: list[JsonDict] = []
    offset = 0
    while True:
        page_params = dict(params)
        page_params["limit"] = page_size
        page_params["offset"] = offset
        payload = run_nova_tool(nova_bin, manifest_path, tool_name, page_params)
        result = payload["result"]
        page_data = result.get("data", [])
        if not isinstance(page_data, list):
            raise RuntimeError(f"{tool_name} returned a non-list data payload for paginated access")
        if not page_data:
            break
        rows.extend(page_data)
        total_available = result.get("total_available", len(rows))
        offset += len(page_data)
        if offset >= total_available:
            break
    return rows


def dump_json(data: Any, output_path: str | None) -> None:
    """Write stable JSON to stdout or a file."""

    text = json.dumps(data, indent=2, sort_keys=True) + "\n"
    if output_path:
        Path(output_path).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)


def dump_text(text: str, output_path: str | None) -> None:
    """Write plain text to stdout or a file."""

    if not text.endswith("\n"):
        text = f"{text}\n"
    if output_path:
        Path(output_path).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)


def dump_csv(rows: list[JsonDict], fieldnames: list[str], output_path: str | None) -> None:
    """Write deterministic CSV to stdout or a file."""

    if output_path:
        handle = Path(output_path).open("w", encoding="utf-8", newline="")
        should_close = True
    else:
        handle = sys.stdout
        should_close = False
    try:
        writer = csv.DictWriter(handle, fieldnames=fieldnames, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow(row)
    finally:
        if should_close:
            handle.close()


def joined(value: Any) -> str:
    """Flatten nested values deterministically for CSV output."""

    if value is None:
        return ""
    if isinstance(value, list):
        return "|".join(str(item) for item in value)
    return str(value)
