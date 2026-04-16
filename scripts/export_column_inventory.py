#!/usr/bin/env python3
"""Export a deterministic Nova column inventory for cleanup and architecture workflows."""

from __future__ import annotations

import argparse
from typing import Any

from _nova_cli import default_nova_bin, dump_csv, dump_json, joined, paginated_tool_rows


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
    parser.add_argument(
        "--role",
        dest="roles",
        action="append",
        default=[],
        help="Repeat to filter by Nova column role.",
    )
    parser.add_argument(
        "--semantic-type",
        dest="semantic_types",
        action="append",
        default=[],
        help="Repeat to filter by Nova semantic type.",
    )
    parser.add_argument(
        "--annotated-only",
        action="store_true",
        help="Include only columns with Nova annotations.",
    )
    parser.add_argument(
        "--page-size",
        type=int,
        default=500,
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


def load_rows(args: argparse.Namespace) -> list[dict[str, Any]]:
    params: dict[str, Any] = {
        "resource_types": sorted(set(args.resource_types)),
        "annotated_only": args.annotated_only,
    }
    if args.roles:
        params["roles"] = sorted(set(args.roles))
    if args.semantic_types:
        params["semantic_types"] = sorted(set(args.semantic_types))
    rows = paginated_tool_rows(
        args.nova_bin,
        args.manifest_path,
        "column_inventory",
        params,
        page_size=args.page_size,
    )
    rows.sort(key=lambda row: (row.get("parent_unique_id", ""), row.get("column_name", "")))
    return rows


def csv_rows(rows: list[dict[str, Any]]) -> list[dict[str, str]]:
    flattened: list[dict[str, str]] = []
    for row in rows:
        flattened.append({key: joined(value) for key, value in row.items()})
    return flattened


def main() -> None:
    args = parse_args()
    rows = load_rows(args)
    if args.format == "json":
        dump_json(rows, args.output)
        return
    fieldnames = [
        "parent_unique_id",
        "parent_name",
        "parent_resource_type",
        "column_name",
        "annotated",
        "role",
        "semantic_type",
        "domains",
        "synonyms",
        "example_values",
        "description",
    ]
    dump_csv(csv_rows(rows), fieldnames, args.output)


if __name__ == "__main__":
    main()
