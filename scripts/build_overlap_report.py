#!/usr/bin/env python3
"""Build a deterministic overlap audit from Nova modelling tools."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from _nova_cli import (
    default_nova_bin,
    dump_json,
    dump_text,
    manifest_identity,
    paginated_tool_rows,
    run_nova_tool,
)


KNOWN_RESOURCE_TYPES = {
    "analysis",
    "doc",
    "exposure",
    "group",
    "macro",
    "metric",
    "model",
    "saved_query",
    "seed",
    "semantic_model",
    "snapshot",
    "source",
    "test",
    "unit_test",
}


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
    args = parser.parse_args()
    if not args.resource_types:
        args.resource_types = ["model"]
    return args


def manifest_summary(args: argparse.Namespace) -> dict[str, Any]:
    metadata = manifest_identity(Path(args.manifest_path))
    return {
        "manifest_identity": metadata,
        "resource_types": sorted(set(args.resource_types)),
    }


def load_overlap_candidates(args: argparse.Namespace) -> tuple[list[dict[str, Any]], int]:
    params: dict[str, Any] = {
        "resource_types": sorted(set(args.resource_types)),
        "limit": args.limit,
        "offset": 0,
    }
    if args.focus_entity:
        params["id_or_name"] = args.focus_entity
        params["resource_type"] = args.focus_resource_type
    payload = run_nova_tool(
        args.nova_bin,
        args.manifest_path,
        "find_entity_overlap",
        params,
    )
    data = payload["result"].get("data", [])
    if not isinstance(data, list):
        raise RuntimeError("find_entity_overlap returned an invalid payload")
    total_available = payload["result"].get("total_available")
    if total_available is None:
        total_available = len(data)
    if not isinstance(total_available, int):
        raise RuntimeError("find_entity_overlap returned an invalid total_available value")
    return data, total_available


def is_absent_resource_type_error(resource_type: str, exc: RuntimeError) -> bool:
    normalized = resource_type.strip().lower()
    if normalized not in KNOWN_RESOURCE_TYPES:
        return False
    message = str(exc).lower()
    return (
        f"resource_type '{normalized}'" in message
        and (
            "is invalid; allowed values:" in message
            or "resolved but was not indexed" in message
        )
    )


def count_entities(args: argparse.Namespace) -> int:
    total = 0
    for resource_type in sorted(set(args.resource_types)):
        try:
            rows = paginated_tool_rows(
                args.nova_bin,
                args.manifest_path,
                "list_entities",
                {"resource_type": resource_type, "detail": "standard"},
            )
        except RuntimeError as exc:
            if is_absent_resource_type_error(resource_type, exc):
                continue
            raise
        total += len(rows)
    return total


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


def inconsistency_sections(clusters: list[dict[str, Any]]) -> dict[str, Any]:
    repeated_indicator_clusters: list[dict[str, Any]] = []
    canonical_conflicts: list[dict[str, Any]] = []
    multi_grain: list[dict[str, Any]] = []
    discovery_risks: list[dict[str, Any]] = []
    for cluster in clusters:
        if cluster["repeated_indicators"]:
            repeated_indicator_clusters.append(
                {
                    "cluster_id": cluster["cluster_id"],
                    "indicators": cluster["repeated_indicators"],
                    "entities": [
                        cluster["entity1"]["unique_id"],
                        cluster["entity2"]["unique_id"],
                    ],
                }
            )
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
        if cluster["entity1"].get("canonical") and cluster["entity2"].get("canonical"):
            canonical_conflicts.append(
                {
                    "cluster_id": cluster["cluster_id"],
                    "entities": [
                        cluster["entity1"]["unique_id"],
                        cluster["entity2"]["unique_id"],
                    ],
                    "reason": "both overlap candidates are marked canonical",
                }
            )
        grain = cluster["grain_comparison"]
        if not grain.get("exact_match", False) or not grain.get("same_time_field", False):
            multi_grain.append(
                {
                    "cluster_id": cluster["cluster_id"],
                    "entities": [
                        cluster["entity1"]["unique_id"],
                        cluster["entity2"]["unique_id"],
                    ],
                    "reason": "overlap candidates do not share the same effective grain",
                }
            )
    return {
        "duplicate_indicators": repeated_indicator_clusters,
        "canonical_conflicts": canonical_conflicts,
        "multi_grain_entities": multi_grain,
        "discovery_risks": discovery_risks,
    }


def cleanup_queue(inconsistencies: dict[str, Any], clusters: list[dict[str, Any]]) -> dict[str, list[str]]:
    immediate: list[str] = []
    for row in inconsistencies["canonical_conflicts"]:
        entity1, entity2 = row.get("entities", ["candidate-1", "candidate-2"])
        immediate.append(f"Resolve canonical conflict between `{entity1}` and `{entity2}`.")
    for row in inconsistencies["duplicate_indicators"]:
        indicators = ", ".join(row.get("indicators", [])) or "repeated indicators"
        immediate.append(f"Review repeated indicator surface for cluster {row.get('cluster_id')}: `{indicators}`.")

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
        later.append(
            f"Normalize grain mismatch in overlap cluster {row.get('cluster_id')}."
        )

    return {
        "immediate": sorted(dict.fromkeys(immediate)),
        "next": sorted(dict.fromkeys(next_actions)),
        "later": sorted(dict.fromkeys(later)),
    }


def unique_displayed_duplicate_indicator_count(inconsistencies: dict[str, Any]) -> int:
    indicators: set[str] = set()
    for row in inconsistencies["duplicate_indicators"]:
        indicators.update(str(indicator) for indicator in row.get("indicators", []))
    return len(indicators)


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    summary = manifest_summary(args)
    candidates, overlap_candidate_count = load_overlap_candidates(args)
    clusters = overlap_clusters(args, candidates)
    inconsistencies = inconsistency_sections(clusters)
    return {
        "scope": summary,
        "summary": {
            "entity_count": count_entities(args),
            "overlap_candidate_count": overlap_candidate_count,
            "displayed_overlap_cluster_count": len(clusters),
            "displayed_duplicate_indicator_count": unique_displayed_duplicate_indicator_count(
                inconsistencies
            ),
            "displayed_canonical_conflict_count": len(inconsistencies["canonical_conflicts"]),
            "displayed_multi_grain_entity_count": len(inconsistencies["multi_grain_entities"]),
            "displayed_discovery_risk_count": len(inconsistencies["discovery_risks"]),
            "inconsistency_count_scope": "displayed_overlap_clusters",
        },
        "overlap_clusters": clusters,
        "inconsistencies": inconsistencies,
        "cleanup_queue": cleanup_queue(inconsistencies, clusters),
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
        f"- Overlap candidates: `{summary['overlap_candidate_count']}`",
        f"- Displayed overlap clusters: `{summary['displayed_overlap_cluster_count']}`",
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
            "## Displayed Inconsistencies",
            f"- scope: `{summary['inconsistency_count_scope']}`",
            f"- duplicate indicators: `{summary['displayed_duplicate_indicator_count']}`",
            f"- canonical conflicts: `{summary['displayed_canonical_conflict_count']}`",
            f"- multi-grain entities: `{summary['displayed_multi_grain_entity_count']}`",
            f"- discovery risks: `{summary['displayed_discovery_risk_count']}`",
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
