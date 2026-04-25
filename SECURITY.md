# Security Policy

## Supported Versions

Security fixes are provided for the latest released version of `dbt-nova`.

## Reporting a Vulnerability

Please report suspected vulnerabilities privately instead of opening a public
issue. Email `joseph.broadhead.dev@gmail.com` with:

- affected version or commit SHA
- environment and deployment mode, if relevant
- reproduction steps or proof of concept
- expected impact

You should receive an acknowledgement within 7 days. Confirmed issues will be
handled with coordinated disclosure, a fix, and release notes when appropriate.

## Scope

Security-sensitive areas include:

- MCP server transports and hosted HTTP deployment behavior
- SQL execution limits, parameter handling, and provider integrations
- manifest/artifact fetching and archive materialization
- installer, release, and reusable workflow supply-chain behavior
- credential handling and logging redaction
