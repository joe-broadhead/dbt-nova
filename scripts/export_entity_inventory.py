#!/usr/bin/env python3
"""Export a deterministic Nova entity inventory for architecture and cleanup workflows."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from _nova_cli import chunked, default_nova_bin, dump_csv, dump_json, joined, paginated_tool_rows, run_nova_tool


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nova-bin", default=default_nova_bin(), help="Path to the dbt-nova binary.")
    parser.add_argument("--manifest-path", required=True, help="Path to manifest.json.")
    parser.add_argument(
        "--resource-type",
        dest="resource_types",
        action="append",
        default=None,
        help="Repeat to include multiple resource types. Defaults to model.",
    )
    parser.add_argument("--package", help="Optional dbt package filter.")
    parser.add_argument(
        "--tag",
        dest="tags",
        action="append",
        default=[],
        help="Repeat to require tags on entities.",
    )
    parser.add_argument("--database-schema", help="Optional database.schema filter.")
    parser.add_argument(
        "--page-size",
        type=int,
        default=200,
        help="Rows to fetch per Nova page.",
    )
    parser.add_argument(
        "--format",
        choices=("csv", "json"),
        default="csv",
        help="Output format.",
    )
    parser.add_argument("--output", help="Write output to a file instead of stdout.")
    args = parser.parse_args()
    if not args.resource_types:
        args.resource_types = ["model"]
    return args


def list_candidate_entities(args: argparse.Namespace) -> list[dict[str, Any]]:
    entities_by_id: dict[str, dict[str, Any]] = {}
    for resource_type in sorted(set(args.resource_types)):
        params: dict[str, Any] = {
            "resource_type": resource_type,
            "detail": "standard",
        }
        if args.package:
            params["package"] = args.package
        if args.tags:
            params["tags"] = sorted(set(args.tags))
        if args.database_schema:
            params["database_schema"] = args.database_schema
        rows = paginated_tool_rows(
            args.nova_bin,
            args.manifest_path,
            "list_entities",
            params,
            page_size=args.page_size,
        )
        for row in rows:
            unique_id = row["unique_id"]
            entities_by_id[unique_id] = row
    return [entities_by_id[key] for key in sorted(entities_by_id)]


def batch_get_entities(args: argparse.Namespace, unique_ids: list[str]) -> list[dict[str, Any]]:
    entities: list[dict[str, Any]] = []
    for batch in chunked(unique_ids, args.page_size):
        payload = run_nova_tool(
            args.nova_bin,
            args.manifest_path,
            "batch_get_entities",
            {
                "unique_ids": batch,
                "detail": "standard",
            },
        )
        result = payload["result"].get("data", {})
        found = result.get("entities", [])
        if not isinstance(found, list):
            raise RuntimeError("batch_get_entities returned an invalid entity list")
        entities.extend(found)
    entities.sort(key=lambda entity: entity["unique_id"])
    return entities


def relation_name(entity: dict[str, Any]) -> str:
    parts = [entity.get("database"), entity.get("schema"), entity.get("alias")]
    return ".".join(str(part) for part in parts if part)


def flatten_entity(entity: dict[str, Any]) -> dict[str, Any]:
    summary = entity.get("nova_summary") or {}
    measures = summary.get("measures") or []
    metrics = summary.get("metrics") or []
    grain = summary.get("grain") or {}
    return {
        "unique_id": entity.get("unique_id"),
        "name": entity.get("name"),
        "resource_type": entity.get("resource_type"),
        "package_name": entity.get("package_name"),
        "database": entity.get("database"),
        "schema": entity.get("schema"),
        "relation_name": relation_name(entity),
        "original_file_path": entity.get("original_file_path"),
        "canonical": bool(summary.get("canonical")),
        "domains": summary.get("domains") or [],
        "use_cases": summary.get("use_cases") or [],
        "synonyms": summary.get("synonyms") or [],
        "time_field": grain.get("time_field"),
        "primary_key": grain.get("primary_key") or [],
        "dimensions": grain.get("dimensions") or [],
        "measure_count": len(measures),
        "metric_count": len(metrics),
        "measure_names": [measure.get("name") for measure in measures if measure.get("name")],
        "metric_names": [metric.get("name") for metric in metrics if metric.get("name")],
    }


def csv_rows(rows: list[dict[str, Any]]) -> list[dict[str, str]]:
    flattened: list[dict[str, str]] = []
    for row in rows:
        flattened.append({key: joined(value) for key, value in row.items()})
    return flattened


def main() -> None:
    args = parse_args()
    listed = list_candidate_entities(args)
    detailed = batch_get_entities(args, [row["unique_id"] for row in listed])
    rows = [flatten_entity(entity) for entity in detailed]
    if args.format == "json":
        dump_json(rows, args.output)
        return
    fieldnames = [
        "unique_id",
        "name",
        "resource_type",
        "package_name",
        "database",
        "schema",
        "relation_name",
        "original_file_path",
        "canonical",
        "domains",
        "use_cases",
        "synonyms",
        "time_field",
        "primary_key",
        "dimensions",
        "measure_count",
        "metric_count",
        "measure_names",
        "metric_names",
    ]
    dump_csv(csv_rows(rows), fieldnames, args.output)


if __name__ == "__main__":
    main()
