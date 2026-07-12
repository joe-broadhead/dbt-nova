/// Canonical MCP tool names exposed by dbt-nova.
pub const MCP_TOOL_NAMES: [&str; 53] = [
    "search",
    "search_indicator",
    "indicator_inventory",
    "search_columns",
    "column_inventory",
    "compare_grains",
    "find_entity_overlap",
    "modelling_consistency_report",
    "get_entity",
    "list_entities",
    "get_lineage",
    "get_sql",
    "get_columns",
    "diff_entities",
    "get_impact",
    "validate_dag",
    "validate_nova_meta",
    "validate_eval_suite",
    "get_eval_gate",
    "get_eval_history",
    "compare_eval_runs",
    "run_eval",
    "init_eval_suite",
    "run_agent_eval",
    "inspect_tool_trace",
    "summarize_tool_trace",
    "redact_tool_trace",
    "replay_tool_trace",
    "show_metadata",
    "health",
    "reload_manifest",
    "warm_manifest",
    "show_config",
    "validate_config",
    "inspect_storage",
    "prune_storage",
    "cleanup_storage",
    "list_tags",
    "list_packages",
    "list_databases",
    "get_column_lineage",
    "get_test_coverage",
    "get_metadata_score",
    "get_metadata_audit",
    "get_agent_readiness",
    "batch_get_entities",
    "find_by_path",
    "search_recipes",
    "get_recipe",
    "run_recipe",
    "get_undocumented",
    "get_context",
    "execute_sql",
];

/// Canonical MCP tool count used by docs/tests.
pub const MCP_TOOL_COUNT: usize = MCP_TOOL_NAMES.len();

/// Default focused tool profile for agent sessions.
pub const DEFAULT_MCP_TOOL_PROFILE: &str = "agent";

/// Supported MCP tool profile names.
pub const MCP_TOOL_PROFILE_NAMES: [&str; 6] =
    ["agent", "analyst", "engineer", "governance", "ops", "all"];

/// Lean default catalog for discovery, context, lineage, quality, and recipe lookup.
pub const MCP_AGENT_TOOL_PROFILE: &[&str] = &[
    "search",
    "search_indicator",
    "indicator_inventory",
    "search_columns",
    "column_inventory",
    "compare_grains",
    "find_entity_overlap",
    "modelling_consistency_report",
    "get_entity",
    "list_entities",
    "get_lineage",
    "get_sql",
    "get_columns",
    "get_impact",
    "validate_dag",
    "show_metadata",
    "health",
    "list_tags",
    "list_packages",
    "list_databases",
    "get_column_lineage",
    "get_test_coverage",
    "get_metadata_score",
    "get_metadata_audit",
    "get_agent_readiness",
    "batch_get_entities",
    "find_by_path",
    "search_recipes",
    "get_recipe",
    "get_undocumented",
    "get_context",
];

/// Analyst profile with read-only discovery plus SQL execution for trusted local sessions.
pub const MCP_ANALYST_TOOL_PROFILE: &[&str] = &[
    "search",
    "search_indicator",
    "indicator_inventory",
    "search_columns",
    "column_inventory",
    "compare_grains",
    "get_entity",
    "list_entities",
    "get_lineage",
    "get_sql",
    "get_columns",
    "get_impact",
    "show_metadata",
    "health",
    "list_tags",
    "list_packages",
    "list_databases",
    "get_column_lineage",
    "get_test_coverage",
    "get_metadata_score",
    "batch_get_entities",
    "find_by_path",
    "search_recipes",
    "get_recipe",
    "get_context",
    "execute_sql",
];

/// Engineer profile adds validation, modelling, recipe execution, and metadata audit tools.
pub const MCP_ENGINEER_TOOL_PROFILE: &[&str] = &[
    "search",
    "search_indicator",
    "indicator_inventory",
    "search_columns",
    "column_inventory",
    "compare_grains",
    "find_entity_overlap",
    "modelling_consistency_report",
    "get_entity",
    "list_entities",
    "get_lineage",
    "get_sql",
    "get_columns",
    "diff_entities",
    "get_impact",
    "validate_dag",
    "validate_nova_meta",
    "show_metadata",
    "health",
    "list_tags",
    "list_packages",
    "list_databases",
    "get_column_lineage",
    "get_test_coverage",
    "get_metadata_score",
    "get_metadata_audit",
    "get_agent_readiness",
    "batch_get_entities",
    "find_by_path",
    "search_recipes",
    "get_recipe",
    "run_recipe",
    "get_undocumented",
    "get_context",
    "execute_sql",
];

/// Governance profile focuses on metadata, quality, readiness, and audit surfaces.
pub const MCP_GOVERNANCE_TOOL_PROFILE: &[&str] = &[
    "search",
    "search_indicator",
    "indicator_inventory",
    "search_columns",
    "column_inventory",
    "compare_grains",
    "find_entity_overlap",
    "modelling_consistency_report",
    "get_entity",
    "list_entities",
    "get_lineage",
    "get_columns",
    "get_impact",
    "validate_dag",
    "validate_nova_meta",
    "show_metadata",
    "health",
    "list_tags",
    "list_packages",
    "list_databases",
    "get_column_lineage",
    "get_test_coverage",
    "get_metadata_score",
    "get_metadata_audit",
    "get_agent_readiness",
    "batch_get_entities",
    "find_by_path",
    "get_undocumented",
    "get_context",
];

/// Operator profile exposes operational, eval, trace, config, and storage tools.
pub const MCP_OPS_TOOL_PROFILE: &[&str] = &[
    "health",
    "show_metadata",
    "reload_manifest",
    "warm_manifest",
    "show_config",
    "validate_config",
    "inspect_storage",
    "prune_storage",
    "cleanup_storage",
    "validate_eval_suite",
    "get_eval_gate",
    "get_eval_history",
    "compare_eval_runs",
    "run_eval",
    "init_eval_suite",
    "run_agent_eval",
    "inspect_tool_trace",
    "summarize_tool_trace",
    "redact_tool_trace",
    "replay_tool_trace",
];

#[must_use]
pub fn mcp_tool_profile_names(profile: &str) -> Option<&'static [&'static str]> {
    match profile {
        "agent" => Some(MCP_AGENT_TOOL_PROFILE),
        "analyst" => Some(MCP_ANALYST_TOOL_PROFILE),
        "engineer" => Some(MCP_ENGINEER_TOOL_PROFILE),
        "governance" => Some(MCP_GOVERNANCE_TOOL_PROFILE),
        "ops" => Some(MCP_OPS_TOOL_PROFILE),
        "all" => Some(&MCP_TOOL_NAMES),
        _ => None,
    }
}

/// Public contract tier for a canonical MCP tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolStability {
    /// Stable fields follow v0.0.x additive-compatibility rules.
    Stable,
    /// Stable fields plus explicit opt-in safety gates for execution or mutation.
    StableGated,
}

impl McpToolStability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::StableGated => "StableGated",
        }
    }
}

/// Machine-readable contract metadata for each canonical MCP tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpToolContract {
    pub tool: &'static str,
    pub stability: McpToolStability,
    pub safety_gate: Option<&'static str>,
    pub docs_anchor: &'static str,
}

macro_rules! mcp_tool_contract {
    ($tool:literal, Stable, $docs_anchor:literal) => {
        McpToolContract {
            tool: $tool,
            stability: McpToolStability::Stable,
            safety_gate: None,
            docs_anchor: $docs_anchor,
        }
    };
    ($tool:literal, StableGated, $safety_gate:expr, $docs_anchor:literal) => {
        McpToolContract {
            tool: $tool,
            stability: McpToolStability::StableGated,
            safety_gate: Some($safety_gate),
            docs_anchor: $docs_anchor,
        }
    };
}

pub const MCP_TOOL_CONTRACTS: [McpToolContract; 53] = [
    mcp_tool_contract!("search", Stable, "#search"),
    mcp_tool_contract!("search_indicator", Stable, "#search_indicator"),
    mcp_tool_contract!("indicator_inventory", Stable, "#indicator_inventory"),
    mcp_tool_contract!("search_columns", Stable, "#search_columns"),
    mcp_tool_contract!("column_inventory", Stable, "#column_inventory"),
    mcp_tool_contract!("compare_grains", Stable, "#compare_grains"),
    mcp_tool_contract!("find_entity_overlap", Stable, "#find_entity_overlap"),
    mcp_tool_contract!(
        "modelling_consistency_report",
        Stable,
        "#modelling_consistency_report"
    ),
    mcp_tool_contract!("get_entity", Stable, "#get_entity"),
    mcp_tool_contract!("list_entities", Stable, "#list_entities"),
    mcp_tool_contract!("get_lineage", Stable, "#get_lineage"),
    mcp_tool_contract!("get_sql", Stable, "#get_sql"),
    mcp_tool_contract!("get_columns", Stable, "#get_columns"),
    mcp_tool_contract!("diff_entities", Stable, "#diff_entities"),
    mcp_tool_contract!("get_impact", Stable, "#get_impact"),
    mcp_tool_contract!("validate_dag", Stable, "#validate_dag"),
    mcp_tool_contract!("validate_nova_meta", Stable, "#validate_nova_meta"),
    mcp_tool_contract!("validate_eval_suite", Stable, "#validate_eval_suite"),
    mcp_tool_contract!("get_eval_gate", Stable, "#get_eval_gate"),
    mcp_tool_contract!("get_eval_history", Stable, "#get_eval_history"),
    mcp_tool_contract!("compare_eval_runs", Stable, "#compare_eval_runs"),
    mcp_tool_contract!(
        "run_eval",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_EVAL_RUN",
        "#run_eval"
    ),
    mcp_tool_contract!(
        "init_eval_suite",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_EVAL_WRITES",
        "#init_eval_suite"
    ),
    mcp_tool_contract!(
        "run_agent_eval",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_AGENT_EVAL; DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER for custom commands",
        "#run_agent_eval"
    ),
    mcp_tool_contract!("inspect_tool_trace", Stable, "#inspect_tool_trace"),
    mcp_tool_contract!(
        "summarize_tool_trace",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_TRACE_WRITES for Markdown report writes",
        "#summarize_tool_trace"
    ),
    mcp_tool_contract!(
        "redact_tool_trace",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_TRACE_WRITES",
        "#redact_tool_trace"
    ),
    mcp_tool_contract!("replay_tool_trace", Stable, "#replay_tool_trace"),
    mcp_tool_contract!("show_metadata", Stable, "#show_metadata"),
    mcp_tool_contract!("health", Stable, "#health"),
    mcp_tool_contract!(
        "reload_manifest",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD for source, refresh, or storage changes",
        "#reload_manifest"
    ),
    mcp_tool_contract!(
        "warm_manifest",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_MANIFEST_WARM",
        "#warm_manifest"
    ),
    mcp_tool_contract!("show_config", Stable, "#show_config"),
    mcp_tool_contract!("validate_config", Stable, "#validate_config"),
    mcp_tool_contract!("inspect_storage", Stable, "#inspect_storage"),
    mcp_tool_contract!(
        "prune_storage",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN",
        "#prune_storage"
    ),
    mcp_tool_contract!(
        "cleanup_storage",
        StableGated,
        "DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN",
        "#cleanup_storage"
    ),
    mcp_tool_contract!("list_tags", Stable, "#list_tags"),
    mcp_tool_contract!("list_packages", Stable, "#list_packages"),
    mcp_tool_contract!("list_databases", Stable, "#list_databases"),
    mcp_tool_contract!("get_column_lineage", Stable, "#get_column_lineage"),
    mcp_tool_contract!("get_test_coverage", Stable, "#get_test_coverage"),
    mcp_tool_contract!("get_metadata_score", Stable, "#get_metadata_score"),
    mcp_tool_contract!("get_metadata_audit", Stable, "#get_metadata_audit"),
    mcp_tool_contract!("get_agent_readiness", Stable, "#get_agent_readiness"),
    mcp_tool_contract!("batch_get_entities", Stable, "#batch_get_entities"),
    mcp_tool_contract!("find_by_path", Stable, "#find_by_path"),
    mcp_tool_contract!("search_recipes", Stable, "#search_recipes"),
    mcp_tool_contract!("get_recipe", Stable, "#get_recipe"),
    mcp_tool_contract!(
        "run_recipe",
        StableGated,
        "DBT_NOVA_TOOL_PROFILE/DBT_NOVA_TOOL_DENYLIST plus SQL provider controls",
        "#run_recipe"
    ),
    mcp_tool_contract!("get_undocumented", Stable, "#get_undocumented"),
    mcp_tool_contract!("get_context", Stable, "#get_context"),
    mcp_tool_contract!(
        "execute_sql",
        StableGated,
        "DBT_NOVA_TOOL_PROFILE/DBT_NOVA_TOOL_DENYLIST plus SQL provider controls",
        "#execute_sql"
    ),
];

#[must_use]
pub fn mcp_tool_contract(tool: &str) -> Option<&'static McpToolContract> {
    MCP_TOOL_CONTRACTS
        .iter()
        .find(|contract| contract.tool == tool)
}

#[must_use]
pub fn mcp_tool_profile_memberships(tool: &str) -> Vec<&'static str> {
    MCP_TOOL_PROFILE_NAMES
        .iter()
        .copied()
        .filter(|profile| {
            mcp_tool_profile_names(profile).is_some_and(|tools| tools.contains(&tool))
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct McpBudgetableDataArrayField {
    pub field: &'static str,
    pub returned_count_field: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct McpResponseBudgetContract {
    pub tool: &'static str,
    pub data_array_fields: &'static [&'static str],
}

pub const MCP_BUDGETABLE_DATA_ARRAY_FIELDS: &[McpBudgetableDataArrayField] = &[
    McpBudgetableDataArrayField {
        field: "columns",
        returned_count_field: None,
    },
    McpBudgetableDataArrayField {
        field: "entities",
        returned_count_field: Some("found_count"),
    },
    McpBudgetableDataArrayField {
        field: "lineage",
        returned_count_field: None,
    },
    McpBudgetableDataArrayField {
        field: "edges",
        returned_count_field: None,
    },
    McpBudgetableDataArrayField {
        field: "not_found",
        returned_count_field: Some("not_found_count"),
    },
    McpBudgetableDataArrayField {
        field: "undocumented_columns",
        returned_count_field: None,
    },
];

pub const MCP_RESPONSE_BUDGET_CONTRACTS: &[McpResponseBudgetContract] = &[
    McpResponseBudgetContract {
        tool: "get_columns",
        data_array_fields: &["columns"],
    },
    McpResponseBudgetContract {
        tool: "batch_get_entities",
        data_array_fields: &["entities", "not_found"],
    },
    McpResponseBudgetContract {
        tool: "get_lineage",
        data_array_fields: &["lineage"],
    },
    McpResponseBudgetContract {
        tool: "get_impact",
        data_array_fields: &["lineage"],
    },
    McpResponseBudgetContract {
        tool: "get_column_lineage",
        data_array_fields: &["edges"],
    },
    McpResponseBudgetContract {
        tool: "get_undocumented",
        data_array_fields: &["entities", "undocumented_columns"],
    },
];

/// CLI/MCP parity state for a top-level CLI leaf command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliMcpParityStatus {
    /// CLI and MCP expose the same product capability today.
    Equivalent,
    /// The command is intentionally outside MCP tool-call parity.
    LifecycleException,
    /// A product capability exists in CLI but does not yet have full MCP parity.
    Gap,
    /// MCP exposes the capability behind an explicit local execution safety gate.
    SafetyGated,
}

/// Checked-in CLI/MCP parity matrix entry.
#[derive(Debug, Clone, Copy)]
pub struct CliMcpParityEntry {
    /// CLI leaf command, using the user-facing command spelling.
    pub cli_command: &'static str,
    /// Current MCP tool that covers the same capability, when one exists.
    pub mcp_tool: Option<&'static str>,
    /// Current parity state.
    pub status: CliMcpParityStatus,
    /// Follow-up issue that owns the gap, if applicable.
    pub issue: Option<&'static str>,
    /// Short rationale for exceptions and gaps.
    pub notes: &'static str,
}

/// Parity contract for every CLI leaf command.
///
/// Keep this matrix in sync with `docs/getting-started/cli.md` and
/// `docs/api/mcp-cli-parity.md`. Product capabilities should trend from `Gap`
/// to `Equivalent`; process lifecycle commands may remain `LifecycleException`.
pub const CLI_MCP_PARITY_MATRIX: [CliMcpParityEntry; 25] = [
    CliMcpParityEntry {
        cli_command: "server start",
        mcp_tool: None,
        status: CliMcpParityStatus::LifecycleException,
        issue: None,
        notes: "starts the MCP process and cannot be called from inside that process",
    },
    CliMcpParityEntry {
        cli_command: "manifest load",
        mcp_tool: Some("health"),
        status: CliMcpParityStatus::LifecycleException,
        issue: None,
        notes: "MCP server startup performs the initial manifest load; health reports the active loaded manifest and reload_manifest replaces it",
    },
    CliMcpParityEntry {
        cli_command: "manifest reload",
        mcp_tool: Some("reload_manifest"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP current-source reload is allowed; source, refresh, or storage changes require DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1",
    },
    CliMcpParityEntry {
        cli_command: "manifest warm",
        mcp_tool: Some("warm_manifest"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP semantic cache warmup requires DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1 and uses the current manifest source",
    },
    CliMcpParityEntry {
        cli_command: "tool call <tool_name>",
        mcp_tool: None,
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "CLI tool-call mode supports the canonical MCP tool catalog",
    },
    CliMcpParityEntry {
        cli_command: "audit agent-readiness",
        mcp_tool: Some("get_agent_readiness"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same agent_readiness.v1 report without CLI file writes",
    },
    CliMcpParityEntry {
        cli_command: "audit metadata-score",
        mcp_tool: Some("get_metadata_audit"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same metadata audit report without CLI file writes or exit semantics",
    },
    CliMcpParityEntry {
        cli_command: "audit nova-meta",
        mcp_tool: Some("validate_nova_meta"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same nova-meta validation report with scoped local path access",
    },
    CliMcpParityEntry {
        cli_command: "config show",
        mcp_tool: Some("show_config"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns active runtime config or defaults without exposing credential env values",
    },
    CliMcpParityEntry {
        cli_command: "config validate",
        mcp_tool: Some("validate_config"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP validates the active runtime config and returns the same structured validation payload",
    },
    CliMcpParityEntry {
        cli_command: "storage inspect",
        mcp_tool: Some("inspect_storage"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same storage inventory payload without mutating storage",
    },
    CliMcpParityEntry {
        cli_command: "storage prune",
        mcp_tool: Some("prune_storage"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP destructive pruning requires DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1",
    },
    CliMcpParityEntry {
        cli_command: "storage cleanup",
        mcp_tool: Some("cleanup_storage"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP destructive cleanup requires DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1",
    },
    CliMcpParityEntry {
        cli_command: "eval init",
        mcp_tool: Some("init_eval_suite"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP file writes require DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1",
    },
    CliMcpParityEntry {
        cli_command: "eval validate",
        mcp_tool: Some("validate_eval_suite"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same eval suite validation data",
    },
    CliMcpParityEntry {
        cli_command: "eval run",
        mcp_tool: Some("run_eval"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP bridge eval execution uses the loaded manifest and requires DBT_NOVA_MCP_ENABLE_EVAL_RUN=1",
    },
    CliMcpParityEntry {
        cli_command: "eval agent run",
        mcp_tool: Some("run_agent_eval"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP provider execution requires DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1; custom commands also require DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1",
    },
    CliMcpParityEntry {
        cli_command: "eval gate",
        mcp_tool: Some("get_eval_gate"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same eval gate report data",
    },
    CliMcpParityEntry {
        cli_command: "eval history",
        mcp_tool: Some("get_eval_history"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns filtered eval telemetry rows in a standard envelope",
    },
    CliMcpParityEntry {
        cli_command: "eval compare",
        mcp_tool: Some("compare_eval_runs"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same local results.json comparison and PR-ready Markdown while scoping local paths under the server working directory",
    },
    CliMcpParityEntry {
        cli_command: "trace inspect",
        mcp_tool: Some("inspect_tool_trace"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP returns the same trace rows, parse warnings, and summary while scoping local paths under the server working directory",
    },
    CliMcpParityEntry {
        cli_command: "trace summarize",
        mcp_tool: Some("summarize_tool_trace"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP returns the same summary data; Markdown report writes require DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1",
    },
    CliMcpParityEntry {
        cli_command: "trace redact",
        mcp_tool: Some("redact_tool_trace"),
        status: CliMcpParityStatus::SafetyGated,
        issue: None,
        notes: "MCP safe-sharing redaction writes require DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1",
    },
    CliMcpParityEntry {
        cli_command: "trace replay",
        mcp_tool: Some("replay_tool_trace"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP replays supported deterministic trace rows against the currently loaded manifest while CLI replay loads an explicit manifest source",
    },
    CliMcpParityEntry {
        cli_command: "health check",
        mcp_tool: Some("health"),
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "both surfaces report manifest/server readiness",
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use clap::{Command as ClapCommand, CommandFactory};

    use crate::cli::args::Cli;

    use super::{
        CLI_MCP_PARITY_MATRIX, CliMcpParityStatus, MCP_AGENT_TOOL_PROFILE,
        MCP_BUDGETABLE_DATA_ARRAY_FIELDS, MCP_RESPONSE_BUDGET_CONTRACTS, MCP_TOOL_CONTRACTS,
        MCP_TOOL_COUNT, MCP_TOOL_NAMES, MCP_TOOL_PROFILE_NAMES, McpToolStability,
        mcp_tool_contract, mcp_tool_profile_memberships, mcp_tool_profile_names,
    };

    #[test]
    fn cli_mcp_parity_matrix_has_unique_cli_commands() {
        let commands = CLI_MCP_PARITY_MATRIX
            .iter()
            .map(|entry| entry.cli_command)
            .collect::<BTreeSet<_>>();

        assert_eq!(commands.len(), CLI_MCP_PARITY_MATRIX.len());
    }

    #[test]
    fn cli_mcp_parity_matrix_covers_cli_leaf_commands() {
        let mut command_paths = Vec::new();
        collect_cli_leaf_commands(&Cli::command(), &[], &mut command_paths);
        let command_paths = command_paths.into_iter().collect::<BTreeSet<_>>();
        let matrix_paths = CLI_MCP_PARITY_MATRIX
            .iter()
            .map(|entry| entry.cli_command.to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(matrix_paths, command_paths);
    }

    #[test]
    fn cli_mcp_parity_matrix_references_valid_mcp_tools() {
        let valid_tools = MCP_TOOL_NAMES.iter().copied().collect::<BTreeSet<_>>();

        for entry in CLI_MCP_PARITY_MATRIX {
            if let Some(tool) = entry.mcp_tool {
                assert!(
                    valid_tools.contains(tool),
                    "{} references unknown MCP tool {tool}",
                    entry.cli_command
                );
            }
            if entry.status == CliMcpParityStatus::Gap {
                assert!(
                    entry.issue.is_some(),
                    "{} is a parity gap without an owning issue",
                    entry.cli_command
                );
            }
        }
    }

    #[test]
    fn tool_profiles_reference_valid_mcp_tools() {
        let valid_tools = MCP_TOOL_NAMES.iter().copied().collect::<BTreeSet<_>>();

        for profile in MCP_TOOL_PROFILE_NAMES {
            let tools = mcp_tool_profile_names(profile)
                .unwrap_or_else(|| panic!("profile {profile} should resolve"));
            assert!(!tools.is_empty(), "profile {profile} must expose tools");
            for tool in tools {
                assert!(
                    valid_tools.contains(tool),
                    "profile {profile} references unknown tool {tool}"
                );
            }
        }
    }

    #[test]
    fn tool_contracts_cover_canonical_catalog_in_order() {
        let contract_tools = MCP_TOOL_CONTRACTS
            .iter()
            .map(|contract| contract.tool)
            .collect::<Vec<_>>();

        assert_eq!(contract_tools, MCP_TOOL_NAMES.to_vec());
        for tool in MCP_TOOL_NAMES {
            assert!(
                mcp_tool_contract(tool).is_some(),
                "missing MCP tool contract for {tool}"
            );
        }
    }

    #[test]
    fn tool_contracts_have_profiles_docs_and_gate_status() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let tools_doc =
            fs::read_to_string(root.join("docs/api/tools.md")).expect("read docs/api/tools.md");

        for contract in MCP_TOOL_CONTRACTS {
            let profiles = mcp_tool_profile_memberships(contract.tool);
            assert!(
                !profiles.is_empty(),
                "{} must be reachable through at least one profile",
                contract.tool
            );

            let heading = format!("### `{}`", contract.tool);
            assert!(
                tools_doc.contains(&heading),
                "docs/api/tools.md must include heading {heading}"
            );
            let stability_row = format!(
                "| [`{}`]({}) | {} |",
                contract.tool,
                contract.docs_anchor,
                contract.stability.as_str()
            );
            assert!(
                tools_doc.contains(&stability_row),
                "docs/api/tools.md must include stability row starting {stability_row}"
            );

            match contract.stability {
                McpToolStability::Stable => assert!(
                    contract.safety_gate.is_none(),
                    "{} is Stable but declares a safety gate",
                    contract.tool
                ),
                McpToolStability::StableGated => assert!(
                    contract.safety_gate.is_some(),
                    "{} is StableGated without a safety gate note",
                    contract.tool
                ),
            }
        }
    }

    #[test]
    fn agent_tool_profile_is_frozen_for_v0_0_x() {
        let frozen_agent_profile = [
            "search",
            "search_indicator",
            "indicator_inventory",
            "search_columns",
            "column_inventory",
            "compare_grains",
            "find_entity_overlap",
            "modelling_consistency_report",
            "get_entity",
            "list_entities",
            "get_lineage",
            "get_sql",
            "get_columns",
            "get_impact",
            "validate_dag",
            "show_metadata",
            "health",
            "list_tags",
            "list_packages",
            "list_databases",
            "get_column_lineage",
            "get_test_coverage",
            "get_metadata_score",
            "get_metadata_audit",
            "get_agent_readiness",
            "batch_get_entities",
            "find_by_path",
            "search_recipes",
            "get_recipe",
            "get_undocumented",
            "get_context",
        ];

        assert_eq!(MCP_AGENT_TOOL_PROFILE, frozen_agent_profile);
    }

    #[test]
    fn agent_tool_profile_is_lean_and_non_operational() {
        let tools = MCP_AGENT_TOOL_PROFILE
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert!(tools.contains("search"));
        assert!(tools.contains("get_context"));
        assert!(tools.contains("search_recipes"));
        assert!(!tools.contains("execute_sql"));
        assert!(!tools.contains("run_eval"));
        assert!(!tools.contains("inspect_storage"));
        assert!(tools.len() < MCP_TOOL_COUNT);
    }

    #[test]
    fn response_budget_contracts_reference_valid_tools_and_fields() {
        let valid_tools = MCP_TOOL_NAMES.iter().copied().collect::<BTreeSet<_>>();
        let budgetable_fields = MCP_BUDGETABLE_DATA_ARRAY_FIELDS
            .iter()
            .map(|field| field.field)
            .collect::<BTreeSet<_>>();
        let contract_tools = MCP_RESPONSE_BUDGET_CONTRACTS
            .iter()
            .map(|contract| contract.tool)
            .collect::<BTreeSet<_>>();

        assert_eq!(contract_tools.len(), MCP_RESPONSE_BUDGET_CONTRACTS.len());
        for contract in MCP_RESPONSE_BUDGET_CONTRACTS {
            assert!(
                valid_tools.contains(contract.tool),
                "budget contract references unknown MCP tool {}",
                contract.tool
            );
            assert!(
                !contract.data_array_fields.is_empty(),
                "budget contract for {} must declare at least one field",
                contract.tool
            );
            for field in contract.data_array_fields {
                assert!(
                    budgetable_fields.contains(field),
                    "budget contract for {} references unregistered field {field}",
                    contract.tool
                );
            }
        }
    }

    #[test]
    fn docs_tool_counts_match_catalog() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let checked_docs = [
            "docs/index.md",
            "docs/getting-started/cli.md",
            "docs/development/architecture.md",
            "docs/api/mcp-cli-parity.md",
            "docs/api/quick-reference.md",
            "docs/api/tools.md",
        ];

        for path in checked_docs {
            let full_path = root.join(path);
            let text = fs::read_to_string(&full_path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", full_path.display()));
            let counts = mentioned_mcp_counts(&text);
            assert!(
                counts.contains(&MCP_TOOL_COUNT),
                "{path} must mention the current MCP tool count {MCP_TOOL_COUNT}; found {counts:?}"
            );
            let stale_counts = counts
                .into_iter()
                .filter(|count| *count != MCP_TOOL_COUNT)
                .collect::<BTreeSet<_>>();
            assert!(
                stale_counts.is_empty(),
                "{path} still contains stale MCP tool counts {stale_counts:?}"
            );
        }
    }

    #[test]
    fn docs_reference_each_mcp_tool() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let checked_docs = ["docs/api/quick-reference.md", "docs/api/tools.md"];

        for path in checked_docs {
            let full_path = root.join(path);
            let text = fs::read_to_string(&full_path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", full_path.display()));
            for tool in MCP_TOOL_NAMES {
                let token = format!("`{tool}`");
                assert!(text.contains(&token), "{path} must mention {token}");
            }
        }
    }

    #[test]
    fn docs_parity_matrix_mentions_each_cli_command_and_issue() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("docs/api/mcp-cli-parity.md");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));

        for entry in CLI_MCP_PARITY_MATRIX {
            let command = format!("`{}`", entry.cli_command);
            assert!(
                text.contains(&command),
                "docs/api/mcp-cli-parity.md must mention {command}"
            );
            if let Some(issue) = entry.issue {
                assert!(
                    text.contains(issue),
                    "docs/api/mcp-cli-parity.md must mention owning issue {issue}"
                );
            }
        }
    }

    fn collect_cli_leaf_commands(
        command: &ClapCommand,
        prefix: &[String],
        leaf_commands: &mut Vec<String>,
    ) {
        let subcommands = command.get_subcommands().collect::<Vec<_>>();
        if subcommands.is_empty() {
            leaf_commands.push(cli_leaf_command_label(prefix));
            return;
        }

        for subcommand in subcommands {
            let mut next_prefix = prefix.to_owned();
            next_prefix.push(subcommand.get_name().to_string());
            collect_cli_leaf_commands(subcommand, &next_prefix, leaf_commands);
        }
    }

    fn cli_leaf_command_label(path: &[String]) -> String {
        let command = path.join(" ");
        if command == "tool call" {
            "tool call <tool_name>".to_string()
        } else {
            command
        }
    }

    fn mentioned_mcp_counts(text: &str) -> BTreeSet<usize> {
        let tokens = text
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        let mut counts = BTreeSet::new();
        for (index, token) in tokens.iter().enumerate() {
            if *token != "MCP" {
                continue;
            }
            let start = index.saturating_sub(3);
            for previous in tokens[start..index].iter().rev() {
                if let Ok(count) = previous.parse::<usize>() {
                    counts.insert(count);
                    break;
                }
            }
        }
        counts
    }
}
