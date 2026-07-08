use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rkyv::string::ArchivedString;
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::config::PersonaWeights;
use crate::config::search::{
    AnalystSemanticConfig, IndicatorRankingConfig, MetadataSupportConfig, SearchConfig,
};
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{
    ArchivedColumnMetaSummary, ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeasure,
    ArchivedNovaMeta, ArchivedNovaMetric,
};
use crate::manifest::search::{
    ManifestSearch, NovaSemanticMatches, SemanticMatchType, match_nova_semantics,
};
use crate::manifest::store::EntityStore;
use crate::manifest::tantivy_search::{SearchHit, SearchRequest, SearchScope, TantivySearcher};
use crate::manifest::vector_search::embedding_text_from_archived;
use crate::params::{
    ColumnInventoryParams, DetailLevel, IndicatorInventoryParams, ListEntitiesParams,
    ParentGroupMode, SearchColumnsParams, SearchIndicatorParams, SearchParams,
};
use crate::responses::{SearchResponse, SuccessResponse};
use crate::utils::{SearchPersona, has_query_syntax, tokenize_alnum_lowercase};
use tracing::{debug, warn};

mod columns;
mod entities;
mod full;
mod indicators;
mod types;

use columns::{
    collect_column_metadata_support_signals, merge_signal_values, normalized_resource_type_filter,
    push_unique_string, resource_type_allowed_for_search,
};
use full::{
    OwnedSearchRequest, SearchDeadline, SemanticWorkResult, bool_to_f32, check_scan_deadline,
    compare_column_inventory_rows, compare_column_search_rows, compare_indicator_inventory_rows,
    compare_indicator_rows, non_zero_option, persona_weights, preferred_grain_for_scoring,
    run_semantic_blocking, run_tantivy_search, usize_to_f32, weighted_rrf_with_explain,
};
use indicators::{
    apply_parent_coherence_bonus, build_search_explain_payload, collect_metadata_support_signals,
    dedupe_indicator_parent_ids, fully_covered_token_count, metadata_support_bonus,
    normalized_indicator_parent_scores, semantic_label_precision_bonus, strip_sql_fields,
    token_overlap_count,
};
use types::{
    ColumnInventoryRow, ColumnSearchMatch, ColumnSearchRow, FusedHitBundle, FusedHitContext,
    IndicatorExecutionMetadata, IndicatorGrainSummary, IndicatorInventoryRow, IndicatorParentGroup,
    IndicatorParentGroupItem, IndicatorScoreExplain, IndicatorSearchContext, IndicatorSearchRow,
    MetadataSupportSignals, ParentIndicatorCoherence, PreparedIndicatorSearch, RetrievalExplain,
    RetrieverContribution, SearchCandidate, SearchExplainConfigSnapshot, SearchExplainPayload,
    SearchScoreContext, SearchScoreExplain, SearchScoreOutcome,
};

#[cfg(test)]
mod tests;
