# Security Policy

## Supported Versions

Security fixes are provided for the latest released version of `dbt-nova`.

| Version | Supported |
| ------- | --------- |
| Latest released version | Yes |
| Older versions | No |

## Reporting a Vulnerability

Please report suspected vulnerabilities privately instead of opening a public
issue. Email `joseph.broadhead.dev@gmail.com` with:

- affected version or commit SHA
- environment and deployment mode, if relevant
- reproduction steps or proof of concept
- expected impact

You should receive an acknowledgement within 7 days. If GitHub private
vulnerability reporting is enabled for this repository, you may use that channel
instead of email.

Confirmed issues are handled with coordinated disclosure, a fix, and release
notes when appropriate.

## Severity And Response Targets

| Severity | Example impact | Target response |
| -------- | -------------- | --------------- |
| Critical | credential exposure, unauthenticated remote code execution, or auth bypass in hosted mode | acknowledge within 7 days and prioritize an urgent patch |
| High | SQL execution safety bypass, artifact integrity bypass, or sensitive data disclosure | acknowledge within 7 days and target the next security patch |
| Medium | denial of service, incorrect security defaults, or limited-scope information leak | triage for the next planned release |
| Low | hardening, documentation, or defense-in-depth improvement | address on the normal maintenance cadence |

## Scope

Security-sensitive areas include:

- MCP server transports and hosted HTTP deployment behavior
- SQL execution limits, parameter handling, and provider integrations
- manifest/artifact fetching and archive materialization
- installer, release, and reusable workflow supply-chain behavior
- credential handling and logging redaction
