    use super::*;

    fn test_indicator_execution_metadata() -> IndicatorExecutionMetadata {
        IndicatorExecutionMetadata {
            indicator_source: "nova_meta",
            execution_surface: "metadata_only",
            queryable: false,
            direct_sql_queryable: false,
            queryable_via: "none",
            execution_note: Some(
                "No deterministic relation or Semantic Layer execution surface is available.",
            ),
        }
    }

    #[test]
    fn candidate_false_multiplier_deboosts_analyst_only_by_default() {
        let config = SearchConfig::default();

        assert!(
            (candidate_false_multiplier(SearchPersona::Analyst, Some(false), false, &config)
                - config.analyst_candidate_false_deboost_factor)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (candidate_false_multiplier(SearchPersona::Engineer, Some(false), false, &config)
                - 1.0)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (candidate_false_multiplier(SearchPersona::Governance, Some(false), false, &config)
                - 1.0)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn candidate_false_multiplier_skips_deboost_for_exact_matches() {
        let config = SearchConfig::default();

        assert!(
            (candidate_false_multiplier(SearchPersona::Analyst, Some(false), true, &config) - 1.0)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn candidate_flag_for_persona_reads_archived_candidates() {
        let nova = crate::manifest::entity::NovaMeta {
            role: None,
            semantic_type: None,
            synonyms: Vec::new(),
            domains: Vec::new(),
            use_cases: Vec::new(),
            example_values: Vec::new(),
            canonical: false,
            tier: None,
            grain: None,
            measures: Vec::new(),
            metric: None,
            metrics: Vec::new(),
            governance: None,
            search: Some(crate::manifest::entity::NovaSearchMeta {
                candidates: Some(crate::manifest::entity::NovaSearchCandidates {
                    analyst: false,
                    engineer: true,
                    governance: true,
                }),
            }),
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&nova).expect("archive nova meta");
        let archived = rkyv::access::<ArchivedNovaMeta, rkyv::rancor::Error>(&bytes)
            .expect("access archived nova meta");

        assert_eq!(
            candidate_flag_for_persona(archived, SearchPersona::Analyst),
            Some(false)
        );
        assert_eq!(
            candidate_flag_for_persona(archived, SearchPersona::Engineer),
            Some(true)
        );
    }

    #[test]
    fn indicator_embedding_text_includes_indicator_and_parent_context() {
        let row = IndicatorSearchRow {
            indicator_name: "average_order_value".to_string(),
            indicator_type: "metric".to_string(),
            canonical: true,
            match_type: "name".to_string(),
            score: 1.0,
            description: Some("Average order value across completed orders".to_string()),
            expression: Some("sum(gmv_amount) / nullif(count(distinct order_id), 0)".to_string()),
            field: None,
            parent_unique_id: "model.pkg.orders_semantic_templates".to_string(),
            parent_name: "orders_semantic_templates".to_string(),
            parent_resource_type: "model".to_string(),
            relation_name: None,
            execution: test_indicator_execution_metadata(),
            domains: vec!["commerce".to_string()],
            grain: IndicatorGrainSummary {
                primary_key: Vec::new(),
                time_field: Some("order_date".to_string()),
                dimensions: vec!["country_code".to_string(), "sales_channel".to_string()],
            },
            support_signals: None,
            explain: None,
        };

        let text = indicator_embedding_text(&row);
        assert!(text.contains("indicator_name: average_order_value"));
        assert!(text.contains("parent_name: orders_semantic_templates"));
        assert!(text.contains("time_field: order_date"));
        assert!(text.contains("domains: commerce"));
    }

    #[test]
    fn reorder_indicator_rows_with_reranker_reorders_top_n_and_preserves_tail() {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "first".to_string(),
                indicator_type: "metric".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 10.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.one".to_string(),
                parent_name: "one".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "second".to_string(),
                indicator_type: "metric".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 9.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.two".to_string(),
                parent_name: "two".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "third".to_string(),
                indicator_type: "metric".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.three".to_string(),
                parent_name: "three".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
        ];

        let reordered = reorder_indicator_rows_with_reranker(
            rows,
            2,
            &[(1, 42.0), (0, 41.0)],
            &SearchConfig::default().indicator_ranking,
        );
        assert_eq!(reordered[0].indicator_name, "second");
        assert!((reordered[0].score - 51.0).abs() < f32::EPSILON);
        assert_eq!(reordered[1].indicator_name, "first");
        assert!((reordered[1].score - 51.0).abs() < f32::EPSILON);
        assert_eq!(reordered[2].indicator_name, "third");
        assert!((reordered[2].score - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reorder_indicator_rows_with_reranker_preserves_strong_prior_winner_when_rerank_gap_is_small()
    {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 10.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.sales".to_string(),
                parent_name: "sales".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "gmv_cancelled".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.5,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.sales".to_string(),
                parent_name: "sales".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
        ];

        let reordered = reorder_indicator_rows_with_reranker(
            rows,
            2,
            &[(1, 1.0), (0, 0.8)],
            &SearchConfig::default().indicator_ranking,
        );
        assert_eq!(reordered[0].indicator_name, "gmv");
        assert!((reordered[0].score - 10.8).abs() < f32::EPSILON);
        assert_eq!(reordered[1].indicator_name, "gmv_cancelled");
        assert!((reordered[1].score - 9.5).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_parent_coherence_bonus_prefers_parent_covering_more_of_question() {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.7,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: vec!["order_id".to_string()],
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec!["gmv".to_string()],
                    domains: vec![],
                    use_cases: vec![],
                    dimensions: vec!["country_code".to_string()],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec!["alpha".to_string()],
                }),
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "net_sales".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.1,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: vec!["order_id".to_string()],
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec![],
                    domains: vec!["commerce".to_string()],
                    use_cases: vec!["revenue_reporting".to_string()],
                    dimensions: vec!["country_code".to_string()],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec!["alpha".to_string()],
                }),
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "promoted_gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 9.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.promotions".to_string(),
                parent_name: "promotions".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: vec!["promotions".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
        ];

        let adjusted =
            apply_parent_coherence_bonus(rows, &SearchConfig::default().indicator_ranking);
        assert_eq!(adjusted[0].parent_unique_id, "model.pkg.fact_orders");
        assert_eq!(adjusted[0].indicator_name, "gmv");
    }

    #[test]
    fn compare_search_candidates_orders_by_final_score_before_parent_signal() {
        let left = SearchCandidate {
            unique_id: "model.pkg.high_score".to_string(),
            entity: None,
            score: 10.0,
            support_signals: None,
            indicator_parent_score: Some(0.2),
            explain: None,
        };
        let right = SearchCandidate {
            unique_id: "model.pkg.low_score".to_string(),
            entity: None,
            score: 8.0,
            support_signals: None,
            indicator_parent_score: Some(0.9),
            explain: None,
        };

        assert_eq!(compare_search_candidates(&left, &right), Ordering::Less);
    }

    #[test]
    fn build_indicator_parent_groups_merges_and_caps_support_signals() {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 10.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec!["gmv".to_string()],
                    domains: vec!["commerce".to_string()],
                    use_cases: vec![],
                    dimensions: vec!["country_code".to_string()],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec!["alpha".to_string(), "beta".to_string()],
                }),
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "net_sales".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "synonym".to_string(),
                score: 9.5,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                execution: test_indicator_execution_metadata(),
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec![],
                    domains: vec![],
                    use_cases: vec!["revenue_reporting".to_string()],
                    dimensions: vec![],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec![
                        "gamma".to_string(),
                        "delta".to_string(),
                        "epsilon".to_string(),
                    ],
                }),
                explain: None,
            },
        ];

        let config = SearchConfig::default();
        let groups = build_indicator_parent_groups(
            &rows,
            &config.indicator_ranking,
            &config.metadata_support,
        );
        assert_eq!(groups.len(), 1);
        let support_signals = groups[0]
            .support_signals
            .as_ref()
            .expect("expected merged support signals");
        assert_eq!(support_signals.domains, vec!["commerce".to_string()]);
        assert_eq!(
            support_signals.use_cases,
            vec!["revenue_reporting".to_string()]
        );
        assert_eq!(support_signals.example_values.len(), 4);
        assert_eq!(support_signals.example_values[0], "alpha");
        assert_eq!(support_signals.example_values[3], "delta");
    }
