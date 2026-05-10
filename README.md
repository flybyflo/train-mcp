# train-mcp

Rust MCP server for Austrian train planning.

What makes it special: the agent does not just fill fixed JSON forms. It writes small JavaScript snippets against a constrained `codemode` API, and `train-mcp` executes that code inside an isolated QuickJS sandbox. That lets the agent chain lookups, inspect intermediate train results, retry with better station matches, and return a shaped answer, while the sandbox only exposes approved train-planning functions and no general network or filesystem access.

It exposes a streamable HTTP MCP endpoint plus a small REST surface for health checks, catalog discovery, metrics, direct tool calls, and optional authenticated query history.

## What is in this repo

- `mcp/`: Rust server crate.
- `mcp/src/server.rs`: MCP tool definitions (`search` and `execute`).
- `mcp/src/executor/`: sandboxed QuickJS CodeMode executor.
- `mcp/src/oebb/` and `mcp/src/transit/`: OEBB API client and transit provider logic.
- `mcp/sql/`: optional Postgres schemas for query history and station lookup data.
- `mcp/bootstrap/`: local Postgres station-data restore hook.
- `mcp/charts/train-mcp/`: Helm chart for deployment.
- `mcp/monitoring/grafana/`: Prometheus/Grafana dashboard support.

There is no mobile or web app in this repository anymore. The repo is scoped to the MCP server and its runtime/deployment support.

## Architecture

```mermaid
flowchart LR
  codex["Codex CLI"]
  claude["Claude Code"]
  rest["REST clients"]

  subgraph server["train-mcp Rust server"]
    http["Axum HTTP router"]
    mcp["/mcp streamable HTTP"]
    api["/api/tools and /catalog"]
    metrics["/metrics"]
    auth["optional JWT auth"]
    codemode["QuickJS CodeMode sandbox"]
    tools["Transit tool dispatcher"]
    provider["OEBB transit provider"]
  end

  oebb["OEBB transport API"]
  postgres[("optional Postgres\nhistory and station lookup")]
  prometheus["Prometheus / Grafana"]
  helm["Docker image + Helm chart"]
  k8s["Kubernetes"]

  codex --> mcp
  claude --> mcp
  rest --> api
  mcp --> http
  api --> http
  http --> auth
  auth --> codemode
  codemode --> tools
  tools --> provider
  provider --> oebb
  provider -. station lookup .-> postgres
  auth -. query history .-> postgres
  metrics --> prometheus
  helm --> k8s
  k8s --> server
```

## Run locally

```bash
cd mcp
cargo run
```

The server listens on `0.0.0.0:${PORT:-8000}`.

Useful local endpoints:

- `POST /mcp`: streamable HTTP MCP endpoint.
- `GET /healthz`: health check.
- `GET /catalog`: JSON catalog of available CodeMode functions.
- `GET /metrics`: Prometheus metrics.
- `GET /api/tools`: REST list of available tools.
- `POST /api/tools/{op_name}`: direct REST call for a transit operation.

## MCP client setup

Claude Code:

```bash
claude mcp add --transport http train-mcp http://127.0.0.1:8000/mcp
```

Codex CLI:

```bash
codex mcp add train-mcp --url http://127.0.0.1:8000/mcp
```

Production endpoint:

```bash
codex mcp add train-mcp --url https://train.floritzmaier.xyz/mcp
```

## Tools

The MCP server exposes two tools:

- `search`: catalog-only sandbox for `codemode.getCatalog({})` and `codemode.listTools({})`.
- `execute`: transit sandbox for `codemode.oebbPlanJourney`, `codemode.oebbPlanTour`, `codemode.oebbResolveItineraryStops`, `codemode.oebbLocations`, `codemode.oebbDepartures`, `codemode.oebbJourneys`, and `codemode.oebbTrip`.

Example `execute` code:

```js
return await codemode.oebbPlanJourney({
  from: "Wien Hbf",
  to: "Linz Hbf",
  departure: "2026-05-10T10:00:00+02:00"
});
```

## Optional Postgres

The server runs without a database. If `DATABASE_URL` is set, it initializes the schemas in `mcp/sql/`, can use station lookup data, and can persist authenticated query history.

Start local Postgres:

```bash
docker compose up -d postgres
```

Use this database URL:

```bash
export DATABASE_URL=postgres://train_mcp:train_mcp@127.0.0.1:55432/train_mcp
```

The compose setup loads:

- `mcp/sql/001_app_schema.sql`
- `mcp/sql/002_transit_schema.sql`
- `mcp/bootstrap/03_station_data.dump`

## Authentication and history

Requests are anonymous by default. To enable per-user query history, set:

```bash
export AUTH_JWT_HS256_SECRET=change-me
export DATABASE_URL=postgres://train_mcp:train_mcp@127.0.0.1:55432/train_mcp
```

Optional JWT constraints:

```bash
export AUTH_JWT_ISSUER=train-mcp
export AUTH_JWT_AUDIENCE=train-mcp-clients
```

Behavior:

- No JWT: requests execute, history is not stored.
- Valid JWT: requests execute and are persisted.
- Invalid JWT: request is rejected with `401`.

Authenticated history endpoints:

- `GET /api/session`
- `GET /api/history/queries?limit=20&offset=0`
- `GET /api/history/journeys?limit=20&offset=0`

## Environment

See `.env.example` for the common local settings.

Core server settings:

- `PORT`: HTTP port, default `8000`.
- `OEBB_BASE_URL`: upstream API, default `https://v6.oebb.transport.rest/api`.
- `MCP_ALLOWED_ORIGINS`: comma-separated CORS allowlist. Empty means no explicit origin allowlist.
- `MCP_MAX_CONCURRENCY`: request concurrency limit, default `128`.
- `MCP_RATE_LIMIT_PER_SECOND`: process-local request rate limit, default `30`.
- `MCP_QUEUE_TIMEOUT_MS`: request queue timeout, default `250`.
- `MCP_MAX_BODY_BYTES`: max request body, default `131072`.

CodeMode sandbox settings:

- `CODEMODE_EXECUTION_TIMEOUT_MS`: JS execution timeout, default `30000`.
- `CODEMODE_EXECUTION_QUEUE_TIMEOUT_MS`: executor queue timeout, default `2000`.
- `CODEMODE_MAX_PARALLEL_EXECUTIONS`: parallel JS executions, default `8`.
- `CODEMODE_MEMORY_LIMIT_BYTES`: QuickJS memory limit, default `33554432`.
- `CODEMODE_MAX_STACK_BYTES`: QuickJS stack limit, default `524288`.
- `CODEMODE_GC_THRESHOLD_BYTES`: QuickJS GC threshold, default `8388608`.

Database settings:

- `DATABASE_URL`: optional Postgres DSN.
- `DB_MAX_CONNECTIONS`: default `10`.
- `DB_ACQUIRE_TIMEOUT_SECONDS`: default `5`.
- `HISTORY_MAX_LIMIT`: maximum history page size, default `100`.

## Development

```bash
cd mcp
cargo fmt
cargo test
```

Build the container image:

```bash
docker build -t train-mcp:local ./mcp
```

Run it:

```bash
docker run --rm -p 8000:8000 --env-file .env train-mcp:local
```

## Deployment

The GitHub Actions workflow builds `./mcp`, pushes it to the local registry configured in `.github/workflows/deploy.yml`, and deploys `mcp/charts/train-mcp` with Helm.

Manual Helm deploy:

```bash
helm upgrade --install train-mcp ./mcp/charts/train-mcp \
  --namespace train \
  --create-namespace \
  --set image.repository=localhost:5000/train-mcp \
  --set image.tag=latest
```
