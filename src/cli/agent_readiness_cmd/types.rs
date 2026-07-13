use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct AgentReadinessReport {
    pub(super) schema_version: &'static str,
    pub(super) generated_at_ms: u128,
    pub(super) manifest: ReadinessManifestSummary,
    pub(super) config: ReadinessConfigSummary,
    pub(super) scoring_contract: JsonValue,
    pub(super) overall_score: u8,
    pub(super) grade: String,
    pub(super) readiness_band: &'static str,
    pub(super) gate_status: &'static str,
    pub(super) summary: ReadinessSummary,
    pub(super) persona_scores: BTreeMap<String, PersonaReadinessScore>,
    pub(super) blocking_findings: Vec<ReadinessFinding>,
    pub(super) improvement_findings: Vec<ReadinessFinding>,
    pub(super) entity_findings: Vec<EntityReadinessFinding>,
    pub(super) indicator_findings: Vec<IndicatorReadinessFinding>,
    pub(super) suggested_meta_patches: Vec<SuggestedMetaPatch>,
    pub(super) golden_question_seeds: Vec<GoldenQuestionSeed>,
    pub(super) eval_status: EvalReadinessStatus,
    pub(super) next_actions: Vec<ReadinessNextAction>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessManifestSummary {
    pub(super) source: String,
    pub(super) hash: String,
    pub(super) version: String,
    pub(super) entity_count: usize,
    pub(super) resource_counts: BTreeMap<String, usize>,
    pub(super) search_ready: ReadinessSearchReady,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessSearchReady {
    pub(super) vector: bool,
    pub(super) sparse: bool,
    pub(super) reranker: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessConfigSummary {
    pub(super) personas: Vec<String>,
    pub(super) resource_types: Vec<String>,
    pub(super) metadata_only: bool,
    pub(super) read_only: bool,
    pub(super) storage_instance_id: String,
    pub(super) thresholds: ReadinessThresholdConfig,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_field_names)]
pub(super) struct ReadinessSummary {
    pub(super) target_count: usize,
    pub(super) scored_count: usize,
    pub(super) blocker_count: usize,
    pub(super) improvement_count: usize,
    pub(super) indicator_count: usize,
    pub(super) ambiguous_indicator_count: usize,
    pub(super) suggested_meta_patch_count: usize,
    pub(super) suggested_meta_patch_required_count: usize,
    pub(super) suggested_meta_patch_recommended_count: usize,
    pub(super) suggested_meta_patch_refinement_count: usize,
    pub(super) suggested_meta_patch_actionable_count: usize,
    pub(super) golden_question_seed_count: usize,
    pub(super) score_buckets: JsonValue,
    pub(super) grade_buckets: JsonValue,
    pub(super) worst_entities_by_persona: JsonValue,
    pub(super) category_weak_spots: JsonValue,
    pub(super) top_recommendation_fields: JsonValue,
    pub(super) drill_down_hints: JsonValue,
    pub(super) agent_modelling: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PersonaReadinessScore {
    pub(super) overall_score: u8,
    pub(super) grade: String,
    pub(super) gate_status: &'static str,
    pub(super) threshold: Option<AppliedThreshold>,
    pub(super) scored_count: usize,
    pub(super) total_available: usize,
    pub(super) quality_summary: JsonValue,
    pub(super) metadata_summary: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessFinding {
    pub(super) severity: &'static str,
    pub(super) category: &'static str,
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) evidence: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EntityReadinessFinding {
    pub(super) unique_id: String,
    pub(super) name: Option<String>,
    pub(super) resource_type: Option<String>,
    pub(super) original_file_path: Option<String>,
    pub(super) overall_score: u8,
    pub(super) grade: String,
    pub(super) persona_scores: BTreeMap<String, u8>,
    pub(super) signals: EntityReadinessSignals,
    pub(super) diagnostics: JsonValue,
    pub(super) recommendations: Vec<ReadinessRecommendation>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct EntityReadinessSignals {
    pub(super) has_description: bool,
    pub(super) has_owner: bool,
    pub(super) has_nova_meta: bool,
    pub(super) has_primary_key: bool,
    pub(super) has_tests: bool,
    pub(super) has_compiled_sql: bool,
    pub(super) column_count: usize,
    pub(super) documented_column_count: usize,
    pub(super) test_count: usize,
    pub(super) upstream_count: usize,
    pub(super) downstream_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessRecommendation {
    pub(super) category: Option<String>,
    pub(super) priority: Option<String>,
    pub(super) impact: Option<u8>,
    pub(super) field: Option<String>,
    pub(super) message: String,
}

pub(super) struct EntityScoreEvidence {
    pub(super) diagnostics: JsonValue,
    pub(super) recommendations: Vec<ReadinessRecommendation>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IndicatorReadinessFinding {
    pub(super) unique_id: String,
    pub(super) name: Option<String>,
    pub(super) resource_type: Option<String>,
    pub(super) indicator_name: Option<String>,
    pub(super) indicator_type: String,
    pub(super) issue: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SuggestedMetaPatch {
    pub(super) id: String,
    pub(super) target_type: &'static str,
    pub(super) unique_id: String,
    pub(super) entity_name: Option<String>,
    pub(super) resource_type: Option<String>,
    pub(super) original_file_path: Option<String>,
    pub(super) column_name: Option<String>,
    pub(super) indicator_name: Option<String>,
    pub(super) indicator_type: Option<String>,
    pub(super) field_path: String,
    pub(super) suggested_value: JsonValue,
    pub(super) placeholder: bool,
    pub(super) rationale: String,
    pub(super) severity: &'static str,
    pub(super) confidence: f32,
    pub(super) evidence: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct GoldenQuestionSeed {
    pub(super) id: String,
    pub(super) seed_type: &'static str,
    pub(super) priority: u8,
    pub(super) persona: &'static str,
    pub(super) question: String,
    pub(super) expected_entities: Vec<String>,
    pub(super) expected_indicators: Vec<String>,
    pub(super) recommended_assertions: Vec<JsonValue>,
    pub(super) rationale: String,
    pub(super) review_required: bool,
    pub(super) date_policy: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EvalReadinessStatus {
    pub(super) status: &'static str,
    pub(super) supplied: bool,
    pub(super) allowed: Option<bool>,
    pub(super) blocked: Option<bool>,
    pub(super) gate_configured: Option<bool>,
    pub(super) threshold: Option<f64>,
    pub(super) pass_rate: Option<f64>,
    pub(super) total_evals: Option<usize>,
    pub(super) failed_evals: Option<usize>,
    pub(super) failed_eval_ids: Vec<String>,
    pub(super) failed_case_ids: Vec<String>,
    pub(super) telemetry_timestamp: Option<String>,
    pub(super) suite_name: Option<String>,
    pub(super) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessNextAction {
    pub(super) priority: u8,
    pub(super) category: &'static str,
    pub(super) action: String,
    pub(super) evidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AppliedThreshold {
    pub(super) min_score: Option<u8>,
    pub(super) min_grade: Option<String>,
    pub(super) severity: ThresholdSeverity,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AppliedCountThreshold {
    pub(super) value: usize,
    pub(super) severity: ThresholdSeverity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum ThresholdSeverity {
    Required,
    #[default]
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ThresholdRule {
    pub(super) min_score: Option<u8>,
    pub(super) min_grade: Option<String>,
    pub(super) severity: ThresholdSeverity,
}

impl Default for ThresholdRule {
    fn default() -> Self {
        Self {
            min_score: None,
            min_grade: None,
            severity: ThresholdSeverity::Advisory,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct CountThresholdRule {
    pub(super) value: usize,
    pub(super) severity: ThresholdSeverity,
}

impl Default for CountThresholdRule {
    fn default() -> Self {
        Self {
            value: 0,
            severity: ThresholdSeverity::Advisory,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(super) struct PersonaThresholdConfig {
    pub(super) default: Option<ThresholdRule>,
    pub(super) analyst: Option<ThresholdRule>,
    pub(super) engineer: Option<ThresholdRule>,
    pub(super) governance: Option<ThresholdRule>,
}

impl PersonaThresholdConfig {
    pub(super) fn rule_for(&self, persona: &str) -> Option<&ThresholdRule> {
        match persona {
            "analyst" => self.analyst.as_ref().or(self.default.as_ref()),
            "engineer" => self.engineer.as_ref().or(self.default.as_ref()),
            "governance" => self.governance.as_ref().or(self.default.as_ref()),
            _ => self.default.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(super) struct ModellingThresholdConfig {
    pub(super) max_blockers: Option<CountThresholdRule>,
    pub(super) max_high: Option<CountThresholdRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(super) struct ReadinessThresholdConfig {
    pub(super) overall: Option<ThresholdRule>,
    pub(super) persona: PersonaThresholdConfig,
    pub(super) modelling: ModellingThresholdConfig,
}

#[derive(Debug, Clone)]
pub(super) struct ReadinessInputs {
    pub(super) personas: Vec<String>,
    pub(super) thresholds: ReadinessThresholdConfig,
    pub(super) eval_status: EvalReadinessStatus,
}

#[derive(Debug, Clone, Default)]
pub(super) struct EntityScoreAccumulator {
    pub(super) name: Option<String>,
    pub(super) resource_type: Option<String>,
    pub(super) persona_scores: BTreeMap<String, u8>,
}

pub(super) struct AgentModellingReadinessResult {
    pub(super) summary: JsonValue,
    pub(super) next_actions: Vec<ReadinessNextAction>,
}

pub(super) struct ReadinessSummaryInput {
    pub(super) target_count: usize,
    pub(super) scored_count: usize,
    pub(super) blocker_count: usize,
    pub(super) improvement_count: usize,
    pub(super) indicator_count: usize,
    pub(super) ambiguous_indicator_count: usize,
    pub(super) suggested_meta_patch_count: usize,
    pub(super) suggested_meta_patch_required_count: usize,
    pub(super) suggested_meta_patch_recommended_count: usize,
    pub(super) suggested_meta_patch_refinement_count: usize,
    pub(super) suggested_meta_patch_actionable_count: usize,
    pub(super) golden_question_seed_count: usize,
    pub(super) triage_summary: ReadinessTriageSummary,
    pub(super) agent_modelling_summary: JsonValue,
}

pub(super) struct NextActionInput<'a> {
    pub(super) overall_score: u8,
    pub(super) eval_status: &'a EvalReadinessStatus,
    pub(super) blocking_findings: &'a [ReadinessFinding],
    pub(super) improvement_findings: &'a [ReadinessFinding],
    pub(super) ambiguous_indicator_count: usize,
    pub(super) suggested_meta_patch_count: usize,
    pub(super) suggested_meta_patch_actionable_count: usize,
    pub(super) suggested_meta_patch_refinement_count: usize,
    pub(super) golden_question_seed_count: usize,
    pub(super) modelling_next_actions: &'a [ReadinessNextAction],
}

pub(super) struct ReadinessFinalSectionInput<'a> {
    pub(super) overall_score: u8,
    pub(super) eval_status: &'a EvalReadinessStatus,
    pub(super) blocking_findings: &'a [ReadinessFinding],
    pub(super) improvement_findings: &'a [ReadinessFinding],
    pub(super) target_count: usize,
    pub(super) scored_count: usize,
    pub(super) persona_scores: &'a BTreeMap<String, PersonaReadinessScore>,
    pub(super) indicator_count: usize,
    pub(super) ambiguous_indicator_count: usize,
    pub(super) suggested_meta_patch_count: usize,
    pub(super) suggested_meta_patch_required_count: usize,
    pub(super) suggested_meta_patch_recommended_count: usize,
    pub(super) suggested_meta_patch_refinement_count: usize,
    pub(super) suggested_meta_patch_actionable_count: usize,
    pub(super) golden_question_seed_count: usize,
    pub(super) agent_modelling: AgentModellingReadinessResult,
}

pub(super) struct ReadinessFinalSections {
    pub(super) readiness_band: &'static str,
    pub(super) gate_status: &'static str,
    pub(super) summary: ReadinessSummary,
    pub(super) next_actions: Vec<ReadinessNextAction>,
}
