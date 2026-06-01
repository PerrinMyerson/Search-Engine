# Security And Privacy Plan

This plan covers the security, privacy, abuse, and compliance work required
before Brutal Browser can credibly claim browser-product readiness, and before
its search mode can credibly claim Google-style search behavior. It is a gate
plan, not proof of implementation.

## Security Standard

- Treat every web page, script, style, image, media file, archive, redirect,
  certificate, query, and crawl response as hostile until proven otherwise.
- No public browser/runtime release without process sandboxing, origin
  isolation, permission boundaries, and a reviewed update path.
- No public search release without query-log minimization, crawl-data retention
  policy, robots/takedown workflow, abuse controls, and observability.
- No benchmark or compatibility claim counts as a security claim unless it is
  tied to a reproducible security gate.

## Assets

- User data: queries, browser history, cookies, storage, downloads, permissions,
  bookmarks, telemetry, local files, and profile secrets.
- Search data: crawl frontier, raw fetch metadata, fetched document snapshots,
  extracted text, index segments, link graph, query logs, rankings, and cached
  renders.
- Browser runtime: renderer process state, DOM, JavaScript heap, network
  requests, cache, storage, compositor surfaces, IPC messages, and crash reports.
- Operations: signing keys, update pipeline, deployment credentials, benchmark
  reports, logs, backups, and incident data.

## Threat Actors

- Malicious websites trying to escape the renderer, steal data, fingerprint
  users, exploit parsers, or abuse permissions.
- Malicious crawled hosts trying to trap, poison, slow, or legally compromise
  the crawler and index.
- Malicious users sending hostile queries, URLs, documents, or benchmark inputs
  to public services.
- Network attackers trying to tamper with TLS, redirects, updates, downloads,
  or dependency supply chains.
- Insider or operational mistakes exposing query logs, user profile data, crawl
  snapshots, or signing credentials.

## Trust Boundaries

| Boundary | Rule | Required Gate |
| --- | --- | --- |
| Web content to renderer | Untrusted content runs without host filesystem or profile access | Sandbox escape tests and filesystem denial tests |
| Renderer to browser UI | Only validated IPC crosses into privileged UI | IPC schema tests, fuzzing, and privilege review |
| Renderer to network | Requests are origin-scoped and policy checked | Origin, CSP, mixed-content, and certificate tests |
| Renderer to storage | Storage is partitioned by origin/site policy | Cookie, local storage, cache, and quota tests |
| Browser profile to telemetry | Telemetry is opt-in and minimized | Privacy review and retention tests |
| Crawler to index | Hostile content cannot corrupt index or exhaust resources | Parser fuzzing, byte caps, timeout tests |
| Query service to logs | Queries are minimized, access-controlled, and retained briefly | Log schema and retention-policy tests |
| Update pipeline to users | Updates are signed, rollback-aware, and auditable | Signature verification and rollback drills |

## Browser Security Requirements

- Origin model: implement scheme/host/port origins, opaque origins for local and
  synthetic documents, same-origin checks, and cross-origin isolation policy.
- Site model: maintain a frame tree with site-instance assignments, origin
  locks, browsing-context groups, COOP/COEP/CORP/CORS decisions, and
  out-of-process iframe boundaries for cross-site documents.
- Process model: separate privileged UI/browser, renderer, GPU, network, and
  storage processes; isolate sites aggressively before executing general
  JavaScript and keep renderer crashes scoped to their tab/frame tree.
- Broker/zygote model: launch sandboxed renderers from a constrained template,
  broker filesystem/device/profile handles, and keep download, file-picker,
  keychain, network, and storage access in privileged services.
- OS sandbox targets: define macOS, Linux, and Windows policies with denied
  filesystem, process, device, environment, network, and IPC capabilities, plus
  explicit allowlists for required shared memory and GPU handles.
- JIT policy: enforce W^X, audit executable memory transitions, partition
  bytecode caches by site/profile, and require per-platform JIT entitlement
  review before enabling optimizing tiers.
- Sandbox: deny filesystem, process spawning, device access, environment
  access, and profile secrets from renderer workers.
- IPC: typed messages, capability-based handles, size limits, timeouts, and
  fuzzed decoders.
- TLS and certificates: platform verification, certificate error interstitials,
  HSTS preload path, revocation strategy, and no silent downgrade.
- Content policy: CSP, mixed-content blocking, referrer policy, permissions
  policy, sandboxed iframes, downloads policy, and pop-up policy.
- Permissions: explicit prompts for geolocation, camera, microphone,
  notifications, clipboard, file access, downloads, and persistent storage.
- Storage privacy: partition cookies/cache/storage, isolate private browsing,
  implement clear-data controls, and define persistence/quota limits.
- Safe downloads and navigation: warn on dangerous file types, blocked schemes,
  deceptive redirects, and unsafe external-app handoff.
- Crash containment: renderer, GPU, network, and storage failures must not grant
  privilege, corrupt profile state, leak private data, or crash unrelated tabs.

## Search Security And Privacy Requirements

- Query privacy: minimize query logs, scrub direct identifiers, enforce
  retention windows, and make telemetry opt-in.
- Crawl legality: respect robots.txt where configured, document exceptions,
  handle takedown and copyright-sensitive cached content, and maintain audit
  records.
- Abuse controls: rate limits, hostile-query detection, URL submission limits,
  crawl trap detection, spam classification, and manual block/allow lists.
- Data minimization: separate raw fetch snapshots from searchable text; expire
  raw content where not needed for reproducible indexing.
- Ranking integrity: detect link farms, duplicate clusters, malware/spam hosts,
  cloaking, and adversarial snippets.
- Public API safety: request limits, response escaping, content security headers,
  auth for mutation/admin endpoints, and structured audit logs.

## Render Worker Requirements

- Workers process untrusted pages with strict CPU, memory, byte, network, and
  wall-clock budgets.
- Workers cannot access user profile storage, local files, host environment, or
  update/signing materials.
- Rendered extraction records must include timeout reason, script/network
  budget usage, final URL, origin, and content hash.
- JS-heavy search extraction must keep the static fast lane isolated from slow
  or hostile render work.

## Security Gates

- Threat model review gate: every public component has assets, actors, trust
  boundaries, and mitigations recorded.
- Sandbox gate: renderer cannot read profile files, spawn processes, open
  arbitrary sockets, or write outside an assigned temp directory.
- Origin gate: same-origin, cross-origin, mixed-content, and local-file tests
  pass.
- Site-isolation gate: OOPIF navigation, COOP/COEP browsing-context swaps,
  CORP/CORS blocking, crash containment, and renderer origin-lock tests pass.
- CSP/permissions gate: policy fixtures match expected allow/block decisions.
- JIT/W^X gate: executable-memory transitions, bytecode-cache partitioning, and
  disabling JIT under policy are tested.
- Parser fuzz gate: HTML/CSS/URL/varint/index parsers survive fuzz corpora and
  resource caps.
- Privacy gate: telemetry is opt-in, query logs are minimized, and retention
  tests enforce deletion windows.
- Abuse gate: crawler detects traps, rate limits hostile hosts, and public APIs
  reject oversized or malicious inputs.
- Update gate: signed update verification, rollback, and key-rotation drills
  pass before packaged browser releases.

## Implementation Sequence

1. Add a security review checklist and make it required for any network-exposed
   command, daemon endpoint, render worker, or browser shell feature.
2. Add origin data structures and tests before expanding navigation or
   JavaScript execution.
3. Split browser runtime into privileged shell, network service, storage
   service, and sandboxed renderer interfaces.
4. Add renderer resource limits and filesystem denial tests.
5. Add CSP, mixed-content, TLS/certificate, and permissions policy fixtures.
6. Add query-log minimization, retention tests, and telemetry opt-in plumbing.
7. Add crawler abuse controls: trap detection, host throttling reports, and
   block/allow policy.
8. Add fuzz harnesses for URL, HTML, CSS, JavaScript bridge, IPC, and index
   parsers.
9. Add signed update and packaging security gates.
10. Promote `Security And Privacy` in `brutal-bench readiness` only when the
    gates above have direct passing evidence.
