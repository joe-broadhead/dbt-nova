use super::{
    ExtendedMetaFieldConfig, ExtendedMetaFieldMode, ExtendedMetaSearchConfig, SearchConfig,
};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_env_vars<R>(vars: &[(&str, Option<&str>)], run: impl FnOnce() -> R) -> R {
    let previous = vars
        .iter()
        .map(|(key, _)| (*key, std::env::var(key).ok()))
        .collect::<Vec<_>>();
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let result = run();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: callers serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    result
}

#[test]
fn default_sparse_batch_size_is_smaller_than_dense_batch_size() {
    let config = SearchConfig::default();
    assert_eq!(config.embedding_batch_size, 128);
    assert_eq!(config.sparse_embedding_batch_size, 16);
}

#[test]
fn semantic_components_are_disabled_by_default() {
    let config = SearchConfig::default();
    assert!(!config.enable_vector_search);
    assert!(!config.enable_sparse_search);
    assert!(!config.enable_reranker);
}

#[test]
fn sparse_batch_size_falls_back_to_general_embedding_batch_size_override() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE", Some("24")),
        ("DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE", None),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = SearchConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.embedding_batch_size, 24);
    assert_eq!(config.sparse_embedding_batch_size, 24);
}

#[test]
fn sparse_specific_batch_size_override_wins_over_general_override() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        ("DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE", Some("24")),
        ("DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE", Some("12")),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = SearchConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.embedding_batch_size, 24);
    assert_eq!(config.sparse_embedding_batch_size, 12);
}

#[test]
fn indicator_ranking_env_overrides_apply() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        (
            "DBT_NOVA_SEARCH_INDICATOR_PARENT_GROUP_MAX_GROUPS",
            Some("7"),
        ),
        (
            "DBT_NOVA_SEARCH_INDICATOR_RERANKER_SCORE_WEIGHT",
            Some("0.25"),
        ),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = SearchConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.indicator_ranking.parent_group_max_groups, 7);
    assert!((config.indicator_ranking.indicator_reranker_score_weight - 0.25).abs() < f32::EPSILON);
}

#[test]
fn metadata_support_env_overrides_apply() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let vars = [
        (
            "DBT_NOVA_SEARCH_METADATA_SUPPORT_MAX_VALUES_PER_FIELD",
            Some("6"),
        ),
        (
            "DBT_NOVA_SEARCH_METADATA_SUPPORT_EXAMPLE_VALUE_WEIGHT",
            Some("0.75"),
        ),
    ];
    let previous = vars.map(|(key, _)| (key, std::env::var(key).ok()));
    for (key, value) in vars {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    let config = SearchConfig::from_env();

    for (key, value) in previous {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::set_var(key, value) };
            }
            None => {
                // SAFETY: tests serialize environment mutation with `ENV_LOCK`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    assert_eq!(config.metadata_support.max_values_per_field, 6);
    assert!((config.metadata_support.example_value_weight - 0.75).abs() < f32::EPSILON);
}

#[test]
fn extended_meta_is_default_off() {
    let config = SearchConfig::default();
    assert!(config.extended_meta.fields.is_empty());
    assert_eq!(config.index_fingerprint(), "");
    config.validate().expect("default config should validate");
}

#[test]
fn extended_meta_env_overrides_apply() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let fields = r#"[
            {
                "path": "meta.owner",
                "alias": "owner",
                "mode": "keyword",
                "boost": 1.25,
                "summary": true
            },
            {
                "path": "columns.*.meta.semantic_group",
                "alias": "semantic_group",
                "mode": "string_array"
            }
        ]"#;
    let vars = [
        ("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON", Some(fields)),
        ("DBT_NOVA_SEARCH_EXTENDED_META_MAX_FIELDS", Some("12")),
        (
            "DBT_NOVA_SEARCH_EXTENDED_META_MAX_VALUES_PER_FIELD",
            Some("9"),
        ),
        (
            "DBT_NOVA_SEARCH_EXTENDED_META_MAX_BYTES_PER_VALUE",
            Some("2048"),
        ),
    ];

    let config = with_env_vars(&vars, SearchConfig::from_env);

    config
        .validate()
        .expect("extended meta config should validate");
    assert_eq!(config.extended_meta.fields.len(), 2);
    assert_eq!(config.extended_meta.fields[0].path, "meta.owner");
    assert_eq!(config.extended_meta.fields[0].alias, "owner");
    assert_eq!(
        config.extended_meta.fields[1].mode,
        ExtendedMetaFieldMode::StringArray
    );
    assert_eq!(config.extended_meta.max_fields, 12);
    assert_eq!(config.extended_meta.max_values_per_field, 9);
    assert_eq!(config.extended_meta.max_bytes_per_value, 2048);
    assert_eq!(config.index_fingerprint().len(), 64);
}

#[test]
fn extended_meta_invalid_mode_env_fails_validation() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let fields = r#"[{"path":"meta.owner","alias":"owner","mode":"number"}]"#;
    let vars = [("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON", Some(fields))];

    let config = with_env_vars(&vars, SearchConfig::from_env);

    let error = config
        .validate()
        .expect_err("invalid extended meta mode should fail validation");
    let message = error.to_string();
    assert!(message.contains("DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON"));
    assert!(message.contains("keyword|text|string_array|bool"));
}

#[test]
fn extended_meta_rejects_sensitive_paths() {
    let config = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            fields: vec![ExtendedMetaFieldConfig {
                path: "meta.accessToken".to_string(),
                alias: "owner".to_string(),
                mode: ExtendedMetaFieldMode::Keyword,
                boost: 1.0,
                summary: false,
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    let error = config
        .validate()
        .expect_err("sensitive extended meta path should fail");
    assert!(error.to_string().contains("token"));
}

#[test]
fn extended_meta_rejects_non_nova_scope_and_discovery_wildcards() {
    let nova_path = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            fields: vec![ExtendedMetaFieldConfig {
                path: "meta.nova.owner".to_string(),
                alias: "owner".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let wildcard_path = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            fields: vec![ExtendedMetaFieldConfig {
                path: "meta.*".to_string(),
                alias: "anything".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(
        nova_path
            .validate()
            .expect_err("meta.nova path should fail")
            .to_string()
            .contains("meta.nova")
    );
    assert!(
        wildcard_path
            .validate()
            .expect_err("schema discovery wildcard should fail")
            .to_string()
            .contains("columns.*.meta")
    );
}

#[test]
fn extended_meta_caps_limit_configured_fields() {
    let config = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            max_fields: 1,
            fields: vec![
                ExtendedMetaFieldConfig {
                    path: "meta.owner".to_string(),
                    alias: "owner".to_string(),
                    ..Default::default()
                },
                ExtendedMetaFieldConfig {
                    path: "meta.domain".to_string(),
                    alias: "domain".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    };

    let error = config
        .validate()
        .expect_err("too many configured fields should fail");
    assert!(error.to_string().contains("max_fields"));
}

#[test]
fn extended_meta_index_fingerprint_is_order_independent_and_changes_with_config() {
    let field_a = ExtendedMetaFieldConfig {
        path: "meta.owner".to_string(),
        alias: "owner".to_string(),
        ..Default::default()
    };
    let field_b = ExtendedMetaFieldConfig {
        path: "columns.*.meta.semantic_group".to_string(),
        alias: "semantic_group".to_string(),
        mode: ExtendedMetaFieldMode::StringArray,
        ..Default::default()
    };
    let config_a = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            fields: vec![field_a.clone(), field_b.clone()],
            ..Default::default()
        },
        ..Default::default()
    };
    let config_b = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            fields: vec![field_b, field_a],
            ..Default::default()
        },
        ..Default::default()
    };
    let config_c = SearchConfig {
        extended_meta: ExtendedMetaSearchConfig {
            max_bytes_per_value: 1024,
            fields: config_a.extended_meta.fields.clone(),
            ..Default::default()
        },
        ..Default::default()
    };

    config_a.validate().expect("config a should validate");
    config_b.validate().expect("config b should validate");
    config_c.validate().expect("config c should validate");
    assert_eq!(config_a.index_fingerprint(), config_b.index_fingerprint());
    assert_ne!(config_a.index_fingerprint(), config_c.index_fingerprint());
}
