use serde_json::{Value as JsonValue, json};

use crate::manifest::search::ManifestSearch;

use super::types::{
    AppliedCountThreshold, AppliedThreshold, CountThresholdRule, ModellingThresholdConfig,
    PersonaReadinessScore, ReadinessFinding, ReadinessThresholdConfig, ThresholdRule,
    ThresholdSeverity,
};

pub(super) fn effective_readiness_thresholds(
    search: &ManifestSearch,
    inputs: &ReadinessThresholdConfig,
) -> ReadinessThresholdConfig {
    let mut thresholds = inputs.clone();
    let modelling = &search.config().agent_readiness.modelling;
    thresholds
        .modelling
        .max_blockers
        .get_or_insert_with(|| CountThresholdRule {
            value: modelling.max_blockers,
            severity: count_threshold_severity(modelling.max_blockers_required),
        });
    thresholds
        .modelling
        .max_high
        .get_or_insert_with(|| CountThresholdRule {
            value: modelling.max_high,
            severity: count_threshold_severity(modelling.max_high_required),
        });
    thresholds
}

pub(super) fn apply_agent_modelling_thresholds(
    summary: &JsonValue,
    thresholds: &ModellingThresholdConfig,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if let Some(rule) = thresholds.max_blockers.as_ref() {
        apply_agent_modelling_count_threshold(
            "blockers",
            "agent_modelling_blocker_threshold_missed",
            summary_usize(summary, "blockers"),
            rule,
            summary,
            blocking_findings,
            improvement_findings,
        );
    }
    if let Some(rule) = thresholds.max_high.as_ref() {
        apply_agent_modelling_count_threshold(
            "high",
            "agent_modelling_high_threshold_missed",
            summary_usize(summary, "high"),
            rule,
            summary,
            blocking_findings,
            improvement_findings,
        );
    }
}

pub(super) fn apply_overall_threshold(
    overall_score: u8,
    grade: &str,
    threshold: Option<&ThresholdRule>,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    let gate = evaluate_threshold(overall_score, grade, threshold);
    if gate == "pass" {
        return;
    }
    let finding = ReadinessFinding {
        severity: threshold_gate_to_finding_severity(gate),
        category: "overall_score",
        code: "overall_threshold_missed",
        message: format!("overall readiness score {overall_score} ({grade}) missed its threshold"),
        evidence: json!({
            "overall_score": overall_score,
            "grade": grade,
            "threshold": threshold.map(applied_threshold)
        }),
    };
    push_threshold_finding(gate, finding, blocking_findings, improvement_findings);
}

pub(super) fn apply_persona_threshold(
    persona: &str,
    score: &PersonaReadinessScore,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if !matches!(score.gate_status, "fail" | "advisory") {
        return;
    }
    let gate = if score.gate_status == "fail" {
        "required_fail"
    } else {
        "advisory_fail"
    };
    let finding = ReadinessFinding {
        severity: threshold_gate_to_finding_severity(gate),
        category: "persona_score",
        code: "persona_threshold_missed",
        message: format!(
            "{persona} readiness score {} ({}) missed its threshold",
            score.overall_score, score.grade
        ),
        evidence: json!({
            "persona": persona,
            "overall_score": score.overall_score,
            "grade": score.grade,
            "threshold": score.threshold
        }),
    };
    push_threshold_finding(gate, finding, blocking_findings, improvement_findings);
}

pub(super) fn evaluate_threshold(
    overall_score: u8,
    grade: &str,
    threshold: Option<&ThresholdRule>,
) -> &'static str {
    let Some(threshold) = threshold else {
        return "pass";
    };
    let score_pass = threshold.min_score.is_none_or(|min| overall_score >= min);
    let grade_pass = threshold
        .min_grade
        .as_deref()
        .is_none_or(|min| grade_meets_threshold(grade, min));
    if score_pass && grade_pass {
        return "pass";
    }
    count_threshold_gate(threshold.severity)
}

pub(super) fn threshold_gate_to_report_gate(gate: &'static str) -> &'static str {
    match gate {
        "required_fail" => "fail",
        "advisory_fail" => "advisory",
        _ => "pass",
    }
}

pub(super) fn applied_threshold(rule: &ThresholdRule) -> AppliedThreshold {
    AppliedThreshold {
        min_score: rule.min_score,
        min_grade: rule.min_grade.clone(),
        severity: rule.severity,
    }
}

fn apply_agent_modelling_count_threshold(
    label: &'static str,
    code: &'static str,
    actual: usize,
    threshold: &CountThresholdRule,
    summary: &JsonValue,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if actual <= threshold.value {
        return;
    }
    let gate = count_threshold_gate(threshold.severity);
    let finding = ReadinessFinding {
        severity: threshold_gate_to_finding_severity(gate),
        category: "modelling",
        code,
        message: format!(
            "agent modelling {label} count {actual} exceeded threshold {}",
            threshold.value
        ),
        evidence: json!({
            "count": actual,
            "threshold": applied_count_threshold(threshold),
            "agent_modelling_summary": summary
        }),
    };
    push_threshold_finding(gate, finding, blocking_findings, improvement_findings);
}

fn count_threshold_severity(required: bool) -> ThresholdSeverity {
    if required {
        ThresholdSeverity::Required
    } else {
        ThresholdSeverity::Advisory
    }
}

fn count_threshold_gate(severity: ThresholdSeverity) -> &'static str {
    match severity {
        ThresholdSeverity::Required => "required_fail",
        ThresholdSeverity::Advisory => "advisory_fail",
    }
}

fn applied_count_threshold(rule: &CountThresholdRule) -> AppliedCountThreshold {
    AppliedCountThreshold {
        value: rule.value,
        severity: rule.severity,
    }
}

fn push_threshold_finding(
    gate: &'static str,
    finding: ReadinessFinding,
    blocking_findings: &mut Vec<ReadinessFinding>,
    improvement_findings: &mut Vec<ReadinessFinding>,
) {
    if gate == "required_fail" {
        blocking_findings.push(finding);
    } else {
        improvement_findings.push(finding);
    }
}

fn threshold_gate_to_finding_severity(gate: &'static str) -> &'static str {
    if gate == "required_fail" {
        "blocker"
    } else {
        "improvement"
    }
}

fn grade_meets_threshold(actual: &str, minimum: &str) -> bool {
    grade_rank(actual) >= grade_rank(minimum)
}

fn grade_rank(grade: &str) -> i8 {
    match grade.trim().to_ascii_uppercase().as_str() {
        "A" => 4,
        "B" => 3,
        "C" => 2,
        "D" => 1,
        _ => 0,
    }
}

fn summary_usize(summary: &JsonValue, key: &str) -> usize {
    summary
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}
