# gemini-mcp-rust documentation index

Central index for this repository's docs. Created 27 Sep 2026. Read this first, then [c4model.md](c4model.md) before any architecture change.

## Naming conventions

| Kind | Pattern | Example |
| --- | --- | --- |
| Dated (a plan, a decision, a point-in-time record) | `DDMMYYYY_topic.md` | `27092026_retry_cache_plan.md` |
| Evergreen (kept current as the code changes) | `topic.md` | `c4model.md` |

Dated docs record what was true or decided on that date. If the facts change later, add a new dated doc and link it rather than rewriting history. Evergreen docs are updated in the same change that alters the behaviour they describe.

## Categories and required sections

| Category | Required sections |
| --- | --- |
| Architecture | Context, Containers, Components, Data flows, Change log |
| Plan (one per task) | Goal, Scope, Decisions, Status, Verification, Deviations |
| Reference | Scope, Facts, Source of truth |

## Index

| Doc | Category | Description |
| --- | --- | --- |
| [c4model.md](c4model.md) | Architecture | Source of truth for the server's architecture: MCP client context, the single binary container, components (MCP server, tools, Gemini client, retry, search cache) and request data flows. |
| [27092026_retry_cache_plan.md](27092026_retry_cache_plan.md) | Plan | Shipping the 23 Dec 2025 retry, search cache, token usage and consensus work: cache key fix, retry hardening, model ID update (checked against Google docs 27 Sep 2026), README corrections and first CI workflow. |
