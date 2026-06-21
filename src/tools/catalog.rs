/// Canonical MCP tool names exposed by dbt-nova.
pub const MCP_TOOL_NAMES: [&str; 43] = [
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
    "run_eval",
    "init_eval_suite",
    "run_agent_eval",
    "show_metadata",
    "health",
    "reload_manifest",
    "warm_manifest",
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
pub const CLI_MCP_PARITY_MATRIX: [CliMcpParityEntry; 20] = [
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
        status: CliMcpParityStatus::Equivalent,
        issue: None,
        notes: "MCP starts a background reload for the live server; CLI reload and CLI tool-call reload are one-shot reloads",
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
        mcp_tool: None,
        status: CliMcpParityStatus::Gap,
        issue: Some("JOE-217"),
        notes: "runtime config inspection is CLI-only today",
    },
    CliMcpParityEntry {
        cli_command: "config validate",
        mcp_tool: None,
        status: CliMcpParityStatus::Gap,
        issue: Some("JOE-217"),
        notes: "runtime config validation is CLI-only today",
    },
    CliMcpParityEntry {
        cli_command: "storage inspect",
        mcp_tool: None,
        status: CliMcpParityStatus::Gap,
        issue: Some("JOE-217"),
        notes: "storage admin inspection is CLI-only today",
    },
    CliMcpParityEntry {
        cli_command: "storage prune",
        mcp_tool: None,
        status: CliMcpParityStatus::Gap,
        issue: Some("JOE-217"),
        notes: "destructive storage admin needs explicit MCP safety gates",
    },
    CliMcpParityEntry {
        cli_command: "storage cleanup",
        mcp_tool: None,
        status: CliMcpParityStatus::Gap,
        issue: Some("JOE-217"),
        notes: "destructive storage admin needs explicit MCP safety gates",
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

    use super::{CLI_MCP_PARITY_MATRIX, CliMcpParityStatus, MCP_TOOL_COUNT, MCP_TOOL_NAMES};

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
    fn docs_tool_counts_match_catalog() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected = format!("{MCP_TOOL_COUNT} MCP");
        let checked_docs = [
            "docs/index.md",
            "docs/getting-started/cli.md",
            "docs/development/architecture.md",
            "docs/api/mcp-cli-parity.md",
        ];

        for path in checked_docs {
            let full_path = root.join(path);
            let text = fs::read_to_string(&full_path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", full_path.display()));
            assert!(
                text.contains(&expected),
                "{path} must mention the current MCP tool count as '{expected}'"
            );
            assert!(
                !text.contains("26 MCP"),
                "{path} still contains stale MCP tool count text"
            );
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
}
