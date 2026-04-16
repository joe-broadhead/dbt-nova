#!/usr/bin/env python3
"""Build a deterministic overlap audit from Nova modelling tools."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from _nova_cli import default_nova_bin, dump_json, dump_text, manifest_identity, run_nova_tool


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nova-bin", default=default_nova_bin(), help="Path to the dbt-nova binary.")
    parser.add_argument("--manifest-path", required=True, help="Path to manifest.json.")
    parser.add_argument(
        "--resource-type",
        dest="resource_types",
        action="append",
        default=["model"],
        help="Repeat to include multiple resource types. Defaults to model.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=10,
        help="Maximum overlap candidates to include in the report.",
    )
    parser.add_argument(
        "--focus-entity",
        help="Optional entity name or unique_id to focus overlap candidates on.",
    )
    parser.add_argument(
        "--focus-resource-type",
        help="Optional resource type for --focus-entity when using a bare name.",
    )
    parser.add_argument(
        "--format",
        choices=("markdown", "json"),
        default="markdown",
        help="Output format.",
    )
    parser.add_argument("--output", help="Write output to a file instead of stdout.")
    return parser.parse_args()


def manifest_summary(args: argparse.Namespace) -> dict[str, Any]:
    metadata = manifest_identity(Path(args.manifest_path))
    return {
        "manifest_identity": metadata,
        "resource_types": sorted(set(args.resource_types)),
    }


def load_consistency_report(args: argparse.Namespace) -> dict[str, Any]:
    payload = run_nova_tool(
        args.nova_bin,
        args.manifest_path,
        "modelling_consistency_report",
        {
            "resource_types": sorted(set(args.resource_types)),
            "limit": args.limit,
        },
    )
    data = payload["result"].get("data")
    if not isinstance(data, dict):
        raise RuntimeError("modelling_consistency_report returned an invalid payload")
    return data


def load_overlap_candidates(args: argparse.Namespace, report: dict[str, Any]) -> list[dict[str, Any]]:
    if not args.focus_entity:
        return list(report.get("overlap_candidates", []))
    payload = run_nova_tool(
        args.nova_bin,
        args.manifest_path,
        "find_entity_overlap",
        {
            "id_or_name": args.focus_entity,
            "resource_type": args.focus_resource_type,
            "resource_types": sorted(set(args.resource_types)),
            "limit": args.limit,
            "offset": 0,
        },
    )
    data = payload["result"].get("data", [])
    if not isinstance(data, list):
        raise RuntimeError("find_entity_overlap returned an invalid payload")
    return data


def load_columns(
    args: argparse.Namespace,
    cache: dict[str, set[str]],
    unique_id: str,
) -> set[str]:
    if unique_id in cache:
        return cache[unique_id]
    payload = run_nova_tool(
        args.nova_bin,
        args.manifest_path,
        "get_columns",
        {"id_or_name": unique_id},
    )
    data = payload["result"].get("data", {})
    columns = data.get("columns", [])
    if not isinstance(columns, list):
        raise RuntimeError("get_columns returned an invalid payload")
    cache[unique_id] = {column["name"] for column in columns if column.get("name")}
    return cache[unique_id]


def compare_grains_for_pair(args: argparse.Namespace, overlap_row: dict[str, Any]) -> dict[str, Any]:
    entity1 = overlap_row["entity1"]
    entity2 = overlap_row["entity2"]
    payload = run_nova_tool(
        args.nova_bin,
        args.manifest_path,
        "compare_grains",
        {
            "entity1": entity1["unique_id"],
            "entity2": entity2["unique_id"],
            "entity1_resource_type": entity1["resource_type"],
            "entity2_resource_type": entity2["resource_type"],
        },
    )
    data = payload["result"].get("data", {})
    if not isinstance(data, dict):
        raise RuntimeError("compare_grains returned an invalid payload")
    return data


def canonical_candidate(overlap_row: dict[str, Any]) -> str | None:
    entity1 = overlap_row["entity1"]
    entity2 = overlap_row["entity2"]
    if entity1.get("canonical") and not entity2.get("canonical"):
        return entity1["unique_id"]
    if entity2.get("canonical") and not entity1.get("canonical"):
        return entity2["unique_id"]
    return None


def shared_business_concept(overlap_row: dict[str, Any]) -> list[str]:
    evidence = overlap_row.get("evidence", {})
    concept: list[str] = []
    for key in (
        "shared_indicators",
        "shared_parent_synonyms",
        "shared_domains",
        "shared_name_tokens",
    ):
        values = evidence.get(key) or []
        if isinstance(values, list):
            concept.extend(str(value) for value in values)
    return sorted(dict.fromkeys(concept))


def overlap_clusters(args: argparse.Namespace, candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    column_cache: dict[str, set[str]] = {}
    clusters: list[dict[str, Any]] = []
    for index, overlap_row in enumerate(candidates, start=1):
        entity1 = overlap_row["entity1"]
        entity2 = overlap_row["entity2"]
        entity1_columns = load_columns(args, column_cache, entity1["unique_id"])
        entity2_columns = load_columns(args, column_cache, entity2["unique_id"])
        grain = compare_grains_for_pair(args, overlap_row)
        repeated_columns = sorted(entity1_columns & entity2_columns)
        clusters.append(
            {
                "cluster_id": index,
                "entity1": entity1,
                "entity2": entity2,
                "score": overlap_row.get("score"),
                "shared_business_concept": shared_business_concept(overlap_row),
                "repeated_columns": repeated_columns,
                "repeated_indicators": sorted(overlap_row.get("evidence", {}).get("shared_indicators", [])),
                "grain_comparison": grain,
                "canonical_candidate": canonical_candidate(overlap_row),
                "evidence": overlap_row.get("evidence", {}),
            }
        )
    return clusters


def inconsistency_sections(report: dict[str, Any], clusters: list[dict[str, Any]]) -> dict[str, Any]:
    duplicate_indicators = list(report.get("duplicate_indicators", []))
    canonical_conflicts = list(report.get("canonical_indicator_conflicts", []))
    multi_grain = list(report.get("entities_with_multiple_grain_variants", []))
    discovery_risks: list[dict[str, Any]] = []
    for cluster in clusters:
        if cluster["canonical_candidate"] is None:
            discovery_risks.append(
                {
                    "cluster_id": cluster["cluster_id"],
                    "entities": [
                        cluster["entity1"]["unique_id"],
                        cluster["entity2"]["unique_id"],
                    ],
                    "reason": "no single canonical candidate is obvious from overlap metadata",
                }
            )
    for row in duplicate_indicators:
        if row.get("canonical_parent_count") != 1:
            discovery_risks.append(
                {
                    "indicator_name": row.get("indicator_name"),
                    "indicator_type": row.get("indicator_type"),
                    "reason": "duplicate indicator is not anchored to exactly one canonical parent",
                }
            )
    return {
        "duplicate_indicators": duplicate_indicators,
        "canonical_conflicts": canonical_conflicts,
        "multi_grain_entities": multi_grain,
        "discovery_risks": discovery_risks,
    }


def cleanup_queue(report: dict[str, Any], inconsistencies: dict[str, Any], clusters: list[dict[str, Any]]) -> dict[str, list[str]]:
    immediate: list[str] = []
    for row in inconsistencies["canonical_conflicts"]:
        immediate.append(
            f"Resolve canonical conflict for {row.get('indicator_type')} `{row.get('indicator_name')}`."
        )
    for row in inconsistencies["duplicate_indicators"]:
        if row.get("inconsistent_grains") or row.get("canonical_parent_count") != 1:
            immediate.append(
                f"Unify duplicate {row.get('indicator_type')} `{row.get('indicator_name')}` across {row.get('parent_count')} parents."
            )

    next_actions: list[str] = []
    for cluster in clusters:
        canonical = cluster["canonical_candidate"]
        if canonical:
            next_actions.append(
                f"Review overlap cluster {cluster['cluster_id']} and keep `{canonical}` as the canonical target."
            )
        else:
            next_actions.append(
                f"Review overlap cluster {cluster['cluster_id']} and choose a canonical target before further cleanup."
            )

    later: list[str] = []
    for row in inconsistencies["multi_grain_entities"]:
        entity = row.get("entity", {})
        later.append(
            f"Normalize grain variants for `{entity.get('unique_id')}` across {row.get('grain_variant_count')} variants."
        )

    return {
        "immediate": sorted(dict.fromkeys(immediate)),
        "next": sorted(dict.fromkeys(next_actions)),
        "later": sorted(dict.fromkeys(later)),
    }


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    summary = manifest_summary(args)
    consistency = load_consistency_report(args)
    candidates = load_overlap_candidates(args, consistency)
    clusters = overlap_clusters(args, candidates)
    inconsistencies = inconsistency_sections(consistency, clusters)
    return {
        "scope": summary,
        "summary": {
            "entity_count": consistency.get("entity_count", 0),
            "overlap_candidate_count": consistency.get("overlap_candidate_count", 0),
            "duplicate_indicator_count": consistency.get("duplicate_indicator_count", 0),
            "canonical_conflict_count": consistency.get("canonical_conflict_count", 0),
            "multi_grain_entity_count": consistency.get("multi_grain_entity_count", 0),
        },
        "overlap_clusters": clusters,
        "inconsistencies": inconsistencies,
        "cleanup_queue": cleanup_queue(consistency, inconsistencies, clusters),
    }


def markdown_report(report: dict[str, Any]) -> str:
    scope = report["scope"]
    summary = report["summary"]
    lines = [
        "# Overlap Audit",
        "",
        "## Scope",
        f"- Manifest path: `{scope['manifest_identity']['manifest_path']}`",
        f"- Manifest sha256: `{scope['manifest_identity']['sha256']}`",
        f"- Resource types: `{', '.join(scope['resource_types'])}`",
        f"- Entity count: `{summary['entity_count']}`",
        "",
        "## Overlap Clusters",
    ]
    for cluster in report["overlap_clusters"]:
        entity1 = cluster["entity1"]
        entity2 = cluster["entity2"]
        grain = cluster["grain_comparison"]
        canonical = cluster["canonical_candidate"] or "review required"
        lines.extend(
            [
                f"- Cluster {cluster['cluster_id']}: `{entity1['unique_id']}` vs `{entity2['unique_id']}`",
                f"  - score: `{cluster['score']}`",
                f"  - shared business concept: `{', '.join(cluster['shared_business_concept']) or 'n/a'}`",
                f"  - repeated columns: `{', '.join(cluster['repeated_columns']) or 'n/a'}`",
                f"  - repeated indicators: `{', '.join(cluster['repeated_indicators']) or 'n/a'}`",
                f"  - exact grain match: `{grain.get('exact_match', False)}`",
                f"  - same time field: `{grain.get('same_time_field', False)}`",
                f"  - shared dimensions: `{', '.join(grain.get('shared_dimensions', [])) or 'n/a'}`",
                f"  - canonical candidate: `{canonical}`",
                f"  - evidence: `{reportable_evidence(cluster['evidence'])}`",
            ]
        )

    lines.extend(
        [
            "",
            "## Inconsistencies",
            f"- duplicate indicators: `{len(report['inconsistencies']['duplicate_indicators'])}`",
            f"- canonical conflicts: `{len(report['inconsistencies']['canonical_conflicts'])}`",
            f"- multi-grain entities: `{len(report['inconsistencies']['multi_grain_entities'])}`",
            f"- discovery risks: `{len(report['inconsistencies']['discovery_risks'])}`",
            "",
            "## Cleanup Queue",
            "- Immediate:",
        ]
    )
    lines.extend(queue_lines(report["cleanup_queue"]["immediate"]))
    lines.append("- Next:")
    lines.extend(queue_lines(report["cleanup_queue"]["next"]))
    lines.append("- Later:")
    lines.extend(queue_lines(report["cleanup_queue"]["later"]))
    return "\n".join(lines)


def reportable_evidence(evidence: dict[str, Any]) -> str:
    parts: list[str] = []
    for key in sorted(evidence):
        value = evidence[key]
        if isinstance(value, list):
            rendered = ", ".join(str(item) for item in value)
        else:
            rendered = str(value)
        parts.append(f"{key}={rendered}")
    return "; ".join(parts) if parts else "n/a"


def queue_lines(items: list[str]) -> list[str]:
    if not items:
        return ["  - n/a"]
    return [f"  - {item}" for item in items]


def main() -> None:
    args = parse_args()
    report = build_report(args)
    if args.format == "json":
        dump_json(report, args.output)
        return
    dump_text(markdown_report(report), args.output)


if __name__ == "__main__":
    main()
