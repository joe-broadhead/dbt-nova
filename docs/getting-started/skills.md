# Agent Skills

Agent Skills are a lightweight, open format for packaging task-specific workflows, references, and optional scripts. Each skill is a directory with a required `SKILL.md` file and optional `scripts/`, `references/`, and `assets/` folders. Agent Skills are designed for progressive disclosure: clients load the name and description first, then load the full instructions only when needed. The `allowed-tools` field is part of the spec but experimental and not supported by every client.

## Skills included in this repo

We ship three persona skills under `.github/skills/`:

- `analyst` — business analysis and reporting workflows
- `engineer` — dbt model development and quality gates
- `governance` — metadata audits and A-grade enforcement

Each skill follows the Agent Skills spec and includes references and templates for deeper guidance.
The analyst skill includes a deterministic metric-resolution workflow:
metric discovery -> entity selection -> time/geo column resolution -> filter-value validation -> final SQL.
The engineer skill mirrors this deterministic pattern:
discovery -> impact analysis -> input validation -> quality gates -> readiness scoring -> ship checklist.
The governance skill now mirrors this deterministic pattern:
preflight -> scope freeze -> paged scoring -> blocker classification -> remediation -> recheck.

## Install in common tools

### Codex (CLI and IDE)

Codex supports Agent Skills for both the CLI and IDE extensions. You can install skills per user or per repository:

- User scope: `~/.codex/skills/<skill-name>`
- Repo scope: `.codex/skills/<skill-name>`

**Recommended (repo scope):**

```bash
mkdir -p .codex/skills
cp -R .github/skills/analyst .codex/skills/analyst
cp -R .github/skills/engineer .codex/skills/engineer
cp -R .github/skills/governance .codex/skills/governance
```

Restart Codex after installing new skills. You can invoke a skill explicitly with `$skill-name` or let Codex select it automatically.

### Claude (Claude apps and Claude Code)

Anthropic supports Agent Skills across Claude apps, Claude Code, and the API. Skills are loaded automatically when relevant. In Claude apps, enable Skills in Settings > Capabilities (Team/Enterprise admins must enable them org-wide) and upload a ZIP file containing your skill folder. For Claude Code, you can install skills via plugins or create a `skills/` directory in your project or plugin root; for personal use you can also add skills to `~/.claude/skills`.

**Recommended (Claude apps):** zip a single skill directory and upload it.

```bash
cd .github/skills
zip -r analyst.skill.zip analyst
```

**Recommended (Claude Code / personal):**

```bash
mkdir -p ~/.claude/skills
cp -R /path/to/repo/.github/skills/analyst ~/.claude/skills/analyst
cp -R /path/to/repo/.github/skills/engineer ~/.claude/skills/engineer
cp -R /path/to/repo/.github/skills/governance ~/.claude/skills/governance
```

### Gemini CLI

Gemini CLI discovers skills from three tiers: workspace (`.gemini/skills/`), user (`~/.gemini/skills/`), and extensions. Workspace skills take precedence over user skills. You can manage skills using `/skills` commands or `gemini skills ...` from the terminal.

**Recommended (workspace scope):**

```bash
mkdir -p .gemini/skills
cp -R .github/skills/analyst .gemini/skills/analyst
cp -R .github/skills/engineer .gemini/skills/engineer
cp -R .github/skills/governance .gemini/skills/governance
```

Then reload skills:

```bash
gemini skills list
gemini skills reload
```

Gemini CLI can also install skills directly from a repo or local path:

```bash
gemini skills install /path/to/skill --scope workspace
```

## Validation (optional)

Use the reference tooling from the Agent Skills project to validate your skills:

```bash
skills-ref validate .github/skills/analyst
```

## Further reading

- Agent Skills specification: https://agentskills.io/specification
- Codex skills docs: https://developers.openai.com/codex/skills
- Claude Skills help center: https://support.claude.com/en/articles/12512180-using-skills-in-claude
- Gemini CLI skills docs: https://geminicli.com/docs/cli/skills/
