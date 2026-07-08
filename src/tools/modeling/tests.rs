use super::report::{overlap_candidate_pairs, overlap_evidence};
use super::*;

fn grain_variant(
    primary_key: &[&str],
    time_field: Option<&str>,
    dimensions: &[&str],
) -> GrainVariant {
    GrainVariant {
        sources: vec!["test".to_string()],
        primary_key: primary_key
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        time_field: time_field.map(str::to_string),
        dimensions: dimensions
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }
}

fn profile_with(
    unique_id: &str,
    column_names: &[&str],
    grain_variants: Vec<GrainVariant>,
) -> EntityOverlapProfile {
    EntityOverlapProfile {
        unique_id: unique_id.to_string(),
        name: unique_id.to_string(),
        resource_type: "model".to_string(),
        relation_name: None,
        canonical: false,
        name_tokens: BTreeSet::new(),
        column_names: column_names
            .iter()
            .map(|value| normalize_value(value))
            .collect(),
        parent_synonyms: BTreeSet::new(),
        domains: BTreeSet::new(),
        indicator_names: BTreeSet::new(),
        typed_indicators: BTreeSet::new(),
        indicator_profiles: BTreeMap::new(),
        column_semantic_types: BTreeSet::new(),
        grain_variants,
    }
}

fn duplicate_indicator_row(name: &str) -> DuplicateIndicatorRow {
    DuplicateIndicatorRow {
        indicator_name: name.to_string(),
        indicator_type: "metric".to_string(),
        parent_count: 2,
        canonical_parent_count: 0,
        parents_without_grain: 0,
        inconsistent_grains: false,
        parents: Vec::new(),
        grain_signatures: Vec::new(),
    }
}

#[test]
fn modelling_next_page_accounts_for_duplicate_sections() {
    let duplicate_rows = vec![
        duplicate_indicator_row("revenue"),
        duplicate_indicator_row("orders"),
    ];

    assert!(modelling_has_next_page(
        1,
        0,
        &[],
        &duplicate_rows,
        &[],
        &[],
    ));
}

fn profile_with_indicator(
    unique_id: &str,
    indicator_type: &str,
    indicator_name: &str,
    canonical: bool,
    entity_grain_variants: Vec<GrainVariant>,
    indicator_grain_variants: Vec<GrainVariant>,
) -> EntityOverlapProfile {
    let mut profile = profile_with(unique_id, &[], entity_grain_variants);
    let key = (indicator_type.to_string(), normalize_value(indicator_name));
    profile.indicator_names.insert(key.1.clone());
    profile.typed_indicators.insert(key.clone());
    profile.indicator_profiles.insert(
        key,
        IndicatorOverlapIndicatorProfile {
            canonical,
            grain_variants: indicator_grain_variants,
        },
    );
    profile
}

fn agent_finding(
    code: &'static str,
    severity: AgentModellingSeverity,
    category: &'static str,
    entity_id: &str,
    indicator_name: &str,
    message: &str,
) -> AgentModellingFinding {
    AgentModellingFinding {
        code,
        severity,
        category,
        message: message.to_string(),
        entities: vec![ModelingEntityRef {
            unique_id: entity_id.to_string(),
            name: entity_id.to_string(),
            resource_type: "model".to_string(),
            relation_name: None,
        }],
        indicators: vec![ModelingIndicatorRef {
            indicator_name: indicator_name.to_string(),
            indicator_type: "metric".to_string(),
            parent_unique_id: entity_id.to_string(),
            source: Some("nova_meta".to_string()),
        }],
        evidence: json!({}),
        recommendation: "Fix the deterministic modelling issue.".to_string(),
        drill_down_hints: Vec::new(),
    }
}

#[test]
fn agent_modelling_summary_counts_and_sorts_buckets() {
    let findings = vec![
        agent_finding(
            "beta_code",
            AgentModellingSeverity::High,
            "queryability",
            "model.pkg.b",
            "beta",
            "beta",
        ),
        agent_finding(
            "alpha_code",
            AgentModellingSeverity::Blocker,
            "grain_safety",
            "model.pkg.a",
            "alpha",
            "alpha",
        ),
        agent_finding(
            "alpha_code",
            AgentModellingSeverity::Medium,
            "grain_safety",
            "model.pkg.c",
            "alpha",
            "alpha duplicate",
        ),
    ];

    let summary = agent_modelling_summary(&findings, true);
    assert_eq!(summary["total"].as_u64(), Some(3));
    assert_eq!(summary["blockers"].as_u64(), Some(1));
    assert_eq!(summary["high"].as_u64(), Some(1));
    assert_eq!(summary["medium"].as_u64(), Some(1));
    assert_eq!(summary["low"].as_u64(), Some(0));
    assert_eq!(summary["truncated"].as_bool(), Some(true));
    assert_eq!(summary["top_codes"][0]["code"].as_str(), Some("alpha_code"));
    assert_eq!(summary["top_codes"][0]["count"].as_u64(), Some(2));
    assert_eq!(
        summary["top_categories"][0]["category"].as_str(),
        Some("grain_safety")
    );
    assert_eq!(summary["top_categories"][0]["count"].as_u64(), Some(2));
}

#[test]
fn agent_modelling_findings_sort_by_contract_order() {
    let mut findings = vec![
        agent_finding(
            "later_low",
            AgentModellingSeverity::Low,
            "queryability",
            "model.pkg.low",
            "low",
            "low",
        ),
        agent_finding(
            "second_blocker",
            AgentModellingSeverity::Blocker,
            "queryability",
            "model.pkg.b",
            "b",
            "second",
        ),
        agent_finding(
            "first_blocker",
            AgentModellingSeverity::Blocker,
            "grain_safety",
            "model.pkg.a",
            "a",
            "first",
        ),
        agent_finding(
            "middle_high",
            AgentModellingSeverity::High,
            "grain_safety",
            "model.pkg.high",
            "high",
            "middle",
        ),
    ];

    sort_agent_modelling_findings(&mut findings);

    let codes = findings
        .iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        vec![
            "first_blocker",
            "second_blocker",
            "middle_high",
            "later_low"
        ]
    );
}

#[test]
fn agent_modelling_findings_truncate_after_sorting() {
    let limit = crate::config::AgentModellingAuditConfig::default().max_findings;
    let mut findings = (0..limit)
        .map(|index| {
            agent_finding(
                "low_code",
                AgentModellingSeverity::Low,
                "queryability",
                &format!("model.pkg.low_{index:03}"),
                "low",
                "low",
            )
        })
        .collect::<Vec<_>>();
    findings.push(agent_finding(
        "blocker_code",
        AgentModellingSeverity::Blocker,
        "queryability",
        "model.pkg.blocker",
        "blocker",
        "blocker",
    ));

    sort_agent_modelling_findings(&mut findings);
    let truncated = findings.len() > limit;
    let bounded = truncate_agent_modelling_findings(&findings, limit);

    assert!(truncated);
    assert_eq!(bounded.len(), limit);
    assert!(matches!(
        bounded[0].severity,
        AgentModellingSeverity::Blocker
    ));
    assert_eq!(bounded[0].code, "blocker_code");
}

#[test]
fn compare_entity_grains_prefers_best_matching_variant() {
    let left = profile_with(
        "model.pkg.left",
        &[],
        vec![
            grain_variant(&["session_id"], Some("session_date"), &["platform_name"]),
            grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code", "sales_channel"],
            ),
        ],
    );
    let right = profile_with(
        "model.pkg.right",
        &[],
        vec![grain_variant(
            &["order_id"],
            Some("order_date"),
            &["country_code", "sales_channel"],
        )],
    );

    let comparison = compare_entity_grains(&left, &right);
    assert!(comparison.exact_match);
    assert!(comparison.same_time_field);
    assert_eq!(comparison.shared_primary_key, vec!["order_id".to_string()]);
    assert_eq!(
        comparison.shared_dimensions,
        vec!["country_code".to_string(), "sales_channel".to_string()]
    );
}

#[test]
fn compare_entity_grains_treats_reordered_fields_as_exact_match() {
    let left = profile_with(
        "model.pkg.left",
        &[],
        vec![grain_variant(
            &["order_id", "line_id"],
            Some("order_date"),
            &["country_code", "sales_channel"],
        )],
    );
    let right = profile_with(
        "model.pkg.right",
        &[],
        vec![grain_variant(
            &["line_id", "order_id"],
            Some("order_date"),
            &["sales_channel", "country_code"],
        )],
    );

    let comparison = compare_entity_grains(&left, &right);
    assert!(comparison.exact_match);
    assert!(comparison.same_time_field);
    assert_eq!(
        comparison.shared_primary_key,
        vec!["line_id".to_string(), "order_id".to_string()]
    );
    assert_eq!(
        comparison.shared_dimensions,
        vec!["country_code".to_string(), "sales_channel".to_string()]
    );
}

#[test]
fn overlap_evidence_includes_shared_column_names() {
    let left = profile_with(
        "model.pkg.left",
        &["order_id", "gmv_amount", "country_code"],
        vec![],
    );
    let right = profile_with(
        "model.pkg.right",
        &["order_id", "gmv_amount", "customer_id"],
        vec![],
    );

    let evidence = overlap_evidence(&left, &right);
    assert_eq!(
        evidence.shared_column_names,
        vec!["gmv_amount".to_string(), "order_id".to_string()]
    );
    assert!(evidence.surface_overlap_count() > 0);
}

#[test]
fn overlap_candidate_pairs_use_shared_column_names() {
    let profiles = vec![
        profile_with("model.pkg.left", &["order_id", "gmv_amount"], vec![]),
        profile_with("model.pkg.right", &["order_id", "gmv_amount"], vec![]),
        profile_with("model.pkg.other", &["promotion_id"], vec![]),
    ];

    let pairs = overlap_candidate_pairs(&profiles, None);
    assert!(pairs.pairs.contains(&(0, 1)));
    assert!(!pairs.pairs.contains(&(0, 2)));
    assert!(!pairs.truncated);
}

#[test]
fn duplicate_indicator_rows_use_indicator_specific_grains() {
    let profiles = vec![
        profile_with_indicator(
            "model.pkg.left",
            "metric",
            "conversion_rate",
            false,
            vec![
                grain_variant(&["session_id"], Some("session_date"), &["platform_name"]),
                grain_variant(&["order_id"], Some("order_date"), &["country_code"]),
            ],
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
        ),
        profile_with_indicator(
            "model.pkg.right",
            "metric",
            "conversion_rate",
            false,
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
        ),
    ];

    let rows = duplicate_indicator_rows(&profiles, 10);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].inconsistent_grains);
    assert_eq!(rows[0].grain_signatures.len(), 1);
}

#[test]
fn duplicate_indicator_rows_count_indicator_level_canonical_flags() {
    let profiles = vec![
        profile_with_indicator(
            "model.pkg.left",
            "measure",
            "gmv",
            true,
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
        ),
        profile_with_indicator(
            "model.pkg.right",
            "measure",
            "gmv",
            false,
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
            vec![grain_variant(
                &["order_id"],
                Some("order_date"),
                &["country_code"],
            )],
        ),
    ];

    let rows = duplicate_indicator_rows(&profiles, 10);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].canonical_parent_count, 1);
    assert_eq!(rows[0].parents.len(), 2);
    assert!(rows[0].parents.iter().any(|parent| parent.canonical));
}
