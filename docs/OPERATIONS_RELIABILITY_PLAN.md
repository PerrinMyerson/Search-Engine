# Operations And Reliability Plan

This plan covers the infrastructure, observability, reliability, rollout, and
incident-response work required before Brutal Browser can credibly operate as a
desktop browser product, and before its search mode can credibly operate as a
public web-search product. It is a gate plan, not proof of production
readiness.

## Operations Standard

- Every service has an owner, health check, metrics, logs, traces, SLO, resource
  budget, backup strategy, and rollback path.
- Every benchmark, crawl, index, and browser-compatibility claim is tied to a
  reproducible corpus, versioned inputs, hardware/OS metadata, and saved report.
- No public service runs without rate limits, structured audit logs, alerting,
  crash reporting, restore drills, and documented incident response.
- No browser package ships without signed artifacts, staged rollout, rollback,
  crash telemetry controls, and update verification.

## Service Topology

| Service | Responsibility | Reliability Requirement |
| --- | --- | --- |
| Crawl frontier | URL state, host budgets, retries, recrawl timestamps | Durable, restartable, host-level visibility |
| Fetch workers | Polite HTTP fetch, robots, redirects, telemetry | Bounded concurrency, backoff, retry budgets |
| Parse/extract workers | HTML text, metadata, links, quality signals | Resource-capped, fuzzed parser inputs |
| Render workers | JS-heavy rendered extraction | Sandboxed, timeout/resource capped |
| Index builders | Segment build, merge, validation, manifest publish | Atomic publish, rollback, corruption checks |
| Shard servers | Hot search over assigned index shards | Health checks, mmap warmup, fanout visibility |
| Query frontends | API/UI search, snippets, render, suggestions | p50/p95/p99 SLOs, rate limits, safe errors |
| Browser test runners | Compatibility, screenshot, performance suites | Reproducible fixtures and artifact retention |
| Benchmark runners | Search/browser gates and readiness reports | Hardware metadata, corpus/index hashes |
| Control plane | Deploys, config, feature flags, rollbacks | Audit log, staged rollout, permissioned access |

## SLOs And Error Budgets

- Search query latency: define p50/p95/p99 targets separately for daemon,
  shard fanout, frontend API, and UI render paths.
- Search freshness: define fetch-to-searchable lag for static pages and a
  separate lag for JS-rendered pages.
- Crawl health: track pages fetched/minute, robots-denied rate, retry rate,
  host backoff rate, trap-detection rate, and successful recrawl ratio.
- Index health: track build duration, segment count, manifest publish latency,
  corruption check failures, rollback count, and shard warmup time.
- Browser reliability: track crash-free render sessions, fixture pass rate,
  screenshot drift, memory high-water marks, and resource timeout rate.
- Operations: track deploy success rate, rollback time, restore time, alert
  acknowledgement time, and incident recurrence.

## Observability

- Metrics: per-service latency histograms, throughput, queue depth, error rate,
  resource usage, cache hit rate, backpressure, and benchmark deltas.
- Logs: structured JSON logs with request/crawl/index IDs, host, shard, segment,
  version, corpus hash, index hash, and sanitized error classes.
- Traces: query frontend to shard fanout, crawl discover/fetch/parse/index
  path, render-worker lifecycle, and benchmark execution.
- Dashboards: search latency, relevance gate health, crawl frontier health,
  recrawl freshness, index build/publish, browser compatibility, cost, and
  incident state.
- Alerts: user-visible search failures, stale index, crawler runaway, shard
  unavailability, corrupt manifest, benchmark regression, crash spike, and
  resource exhaustion.

## Backup And Restore

- Frontier snapshots: frequent durable snapshots plus write-ahead event logs for
  URL state, host state, retries, and recrawl timestamps.
- Raw crawl metadata: versioned storage for HTTP metadata, hashes, redirects,
  and fetch telemetry with retention policy.
- Index artifacts: immutable segment storage, manifest history, corpus/index
  hashes, checksums, and atomic current-manifest pointer.
- Evaluation artifacts: query sets, judgments, benchmark reports, browser
  fixtures, screenshot baselines, and readiness reports.
- User/browser data: profile backup and export/delete policy only after privacy
  controls are implemented.
- Restore drills: scripted restore into a fresh environment, verify hash
  integrity, warm shards, replay canary queries, and publish report.

## Deployment And Rollback

- Environments: local, fixture CI, staging with replay corpora, large-scale
  benchmark environment, and production.
- Releases: versioned binaries, config manifests, schema/index version checks,
  feature flags, staged rollout, and automatic rollback triggers.
- Index publish: build into a new immutable version, validate, warm, run canary
  queries, then atomically promote.
- Browser packages: signed artifacts, update signature verification,
  rollback-safe update channel, and crash/telemetry opt-in controls.
- Configuration: typed config, diff review, audit log, secrets separation, and
  emergency disable switches for crawl, render workers, and public APIs.

## Failure Injection

- Crawler restart during frontier claims, robots fetch failures, DNS failures,
  redirect loops, crawl traps, slow hosts, and hostile content.
- Index builder crash during segment write, manifest corruption, partial shard
  warmup, stale shard, and incompatible segment version.
- Query frontend timeout, shard fanout partial failure, cache outage, malformed
  query, oversized request, and invalid render target.
- Render worker timeout, memory cap, hostile script, network cap, and sandbox
  denial.
- Browser fixture runner crash, screenshot mismatch, Chrome baseline missing,
  and performance regression.
- Deployment rollback, backup restore, alert storm, credential rotation, and
  dependency outage.

## Capacity And Cost Controls

- Per-host crawl budgets, global crawl budgets, render-worker budget, API rate
  limits, shard memory budget, cache budget, and benchmark runtime budget.
- Cost dashboards for crawl bandwidth, storage, compute, render-worker time,
  benchmark runs, logs/traces, and backup retention.
- Admission controls for URL submissions, recrawl requests, render jobs,
  benchmark jobs, and large local corpora.

## Incident Response

- Severity definitions for search outage, stale index, data exposure, crawler
  abuse, browser security issue, update failure, and benchmark regression.
- On-call runbooks for crawl stop, index rollback, shard evacuation, API rate
  limiting, render-worker disable, update rollback, and data retention hold.
- Post-incident review template with timeline, customer impact, root cause,
  missed detection, action items, owner, and verification gate.

## Operations Gates

- Local pipeline gate: documented commands start crawl/index/search/render/UI
  and emit health/benchmark reports.
- Staging replay gate: controlled corpus replay builds a fresh index, warms
  shards, runs canary queries, and publishes p50/p95/p99 plus freshness report.
- Restore gate: frontier/index/evaluation artifacts restore into a clean
  environment and pass integrity and canary checks.
- Failure-injection gate: selected failure scenarios above run automatically and
  produce expected degraded behavior.
- Load gate: query frontend and shard fanout meet p50/p95/p99 SLOs under
  controlled load.
- Observability gate: dashboards and alerts exist for every public service and
  benchmark gate.
- Release gate: staged deploy, rollback, signed browser package, and config
  audit checks pass.
- Cost gate: crawl, render, storage, benchmark, and serving budgets are enforced
  and visible.

## Implementation Sequence

1. Define service config schemas for crawl, index, shard, query, render, bench,
   and browser-test runners.
2. Add structured logs and metrics IDs to every crawl/index/query/render path.
3. Add health endpoints and machine-readable status reports for each service.
4. Add restore scripts for frontier snapshots and immutable index artifacts.
5. Add failure-injection fixtures for crawler, index, shard, query, and render
   workers.
6. Add staging replay pipeline using a fixed corpus and canary queries.
7. Add dashboards and alert rules from the SLO list.
8. Add deploy/rollback runbooks and typed release manifests.
9. Add signed browser/CLI package pipeline and update rollback drills.
10. Promote `Operations And Reliability` in `brutal-bench readiness` only when
    the gates above have direct passing evidence.
