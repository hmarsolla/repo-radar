# repo-radar — Task Breakdown

**Companion to:** [PRD.md](PRD.md) (requirements) and [DESIGN.md](DESIGN.md) (technical design)
**Last updated:** 2026-09-02

---

## How to use this file

Tasks are grouped by milestone and identified as `M<milestone>-<n>`. Each carries:

- **Refs** — the FR numbers and DESIGN sections that specify it. Read those before starting; this file is a checklist, not a spec.
- **Deps** — tasks that must land first.
- **Size** — S (< half a day), M (~1 day), L (multiple days).
- **Done when** — the acceptance criteria. A task is not done because code exists; it is done when these hold.

Milestones are sequential, but tasks within a milestone can often run in parallel. Two orderings matter and are called out below: the **critical path** (§ Critical path) and **M2-12**, a spike that must run before the rest of M2 is built on top of it.

---

## P — Prerequisites

- [ ] **P-1 · Install the Rust toolchain** · Size: S · Deps: none
  Refs: DESIGN §8 (prerequisites), PRD §8

  `rustup` with stable. Windows additionally needs **Microsoft C++ Build Tools (MSVC)** — required by `rusqlite`'s bundled SQLite, `git2`'s libgit2, and Tauri itself. Node 24.11.1 is already present.

  **Done when:** `cargo --version`, `rustc --version`, and `rustup --version` all resolve, and `cargo new --bin /tmp/probe && cargo build` succeeds in that probe (proving the C toolchain links, not just that rustc exists).

- [ ] **P-2 · Verify Tauri system dependencies for the dev platform** · Size: S · Deps: P-1

  **Done when:** `cargo tauri info` (or `npx tauri info`) reports no missing dependencies.

---

## M0 — Scaffold

Goal: an empty but correctly-structured app that builds on all three platforms, with the database, bindings, and CI in place. No analysis logic.

- [ ] **M0-1 · Initialize workspace and Tauri app** · Size: M · Deps: P-1, P-2
  Refs: DESIGN §2

  Cargo workspace at the root with `crates/core` and `src-tauri` as members. Tauri 2 app with the React + TypeScript + Vite template. Directory tree per DESIGN §2.

  **Done when:** `cargo build` succeeds at the workspace root, `npm run tauri dev` opens a window, and the tree matches DESIGN §2.

- [ ] **M0-2 · Frontend styling foundation** · Size: S · Deps: M0-1
  Refs: PRD §8

  Tailwind CSS, shadcn/ui initialized, dark/light/system theme with a working toggle.

  **Done when:** A shadcn component renders and the theme toggle switches correctly, including on system-preference change.

- [ ] **M0-3 · Core crate skeleton and layering** · Size: S · Deps: M0-1
  Refs: DESIGN §3, §4

  `crates/core` with the module tree from DESIGN §2 and the domain types from DESIGN §4. `Paths` struct (DESIGN §13.1).

  **Done when:** `cargo tree -p repo-radar-core` shows **no `tauri` dependency**. This is the load-bearing constraint of the whole design (DESIGN §3) — verify it, don't assume it.

- [ ] **M0-4 · Database layer** · Size: M · Deps: M0-3
  Refs: DESIGN §5

  WAL mode and pragmas, `r2d2_sqlite` read pool (4) plus a dedicated write connection behind a `Mutex`, `include_str!` migration runner with a `schema_version` table, and migration `0001` containing the full schema and indexes from DESIGN §5.3–5.4.

  **Done when:** Migrations apply to a fresh file and are idempotent on re-open; a test opens an in-memory DB, applies migrations, and round-trips a row through each table.

- [ ] **M0-5 · Generated TypeScript bindings** · Size: M · Deps: M0-1, M0-3
  Refs: DESIGN §12.1, §19

  `specta` + `tauri-specta` emitting `src/bindings.ts` at build time. One trivial command wired end to end to prove the pipeline.

  **Done when:** Calling the trivial command from React type-checks against generated types, and a deliberate Rust type change produces a corresponding TypeScript change without hand-editing. Add the CI dirty-check in M0-8.

- [ ] **M0-6 · App shell** · Size: M · Deps: M0-2, M0-5
  Refs: PRD §6, DESIGN §14.2

  React Router with routes for Dashboard, Repos, Repo detail, Advisories, Settings. Feature-first directory layout. Persistent slots in the layout for the scan progress indicator and advisory freshness indicator (populated later).

  **Done when:** All routes navigate and render placeholders; layout is responsive.

- [ ] **M0-7 · Logging and error tiers** · Size: S · Deps: M0-3
  Refs: DESIGN §15

  `tracing` with a local file appender in the data dir. `thiserror` error enums per module. The three-tier taxonomy from DESIGN §15 expressed as types, including the `Warning` type (DESIGN §4.1).

  **Done when:** Logs write to the data dir; `Warning` serializes to the frontend.

- [ ] **M0-8 · CI** · Size: M · Deps: M0-1, M0-5
  Refs: DESIGN §19

  GitHub Actions matrix over `windows-latest`, `macos-latest`, `ubuntu-latest`: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `tsc --noEmit`, `eslint`, `tauri build`. Plus a check that regenerating bindings leaves the tree clean.

  **Done when:** All three platforms pass green, and a deliberate uncommitted-bindings change fails CI.

- [ ] **M0-9 · Settings persistence and path injection** · Size: M · Deps: M0-4, M0-5
  Refs: FR-10.1, FR-10.2, DESIGN §13.1

  Tauri store plugin for config; `Paths` constructed in `src-tauri` and injected into core. `get_settings` / `set_settings` / `add_scan_root` / `remove_scan_root` commands, with path validation on add.

  **Done when:** Settings survive a restart; the DB lands in the OS data dir and config in the OS config dir; **nothing is written into the source tree at runtime** (FR-10.2).

---

## M1 — Discovery

Goal: point the app at a folder and see real repositories with real git and language data, streaming in as they complete.

- [ ] **M1-1 · Repository discovery walker** · Size: L · Deps: M0-3
  Refs: FR-1.1–1.4, FR-1.6–1.8, DESIGN §6.6

  `ignore`-based walk. Stop descending at `.git` (FR-1.3). Prune list (FR-1.4). No symlink following (FR-1.8). Handle `.git` as a directory, as a worktree pointer file (FR-1.7), and bare repos (FR-1.6).

  **Done when:** Against a fixture tree containing a normal repo, a nested repo inside `node_modules`, a worktree, a bare repo, and a symlink cycle: exactly the expected repos are found, the `node_modules` repo is **not** among them, and the walk terminates.

- [ ] **M1-2 · Submodule handling** · Size: M · Deps: M1-1
  Refs: FR-1.5, DESIGN §6.6

  Read `.gitmodules` from the parent after opening it; attach submodules as children via `parent_repo_id`. Their dependencies are attributed to the parent's `repo_id`.

  **Done when:** A fixture repo with a submodule yields one top-level row; the submodule appears in the parent's detail and never in the repo list.

- [ ] **M1-3 · Git metadata extraction** · Size: L · Deps: M0-3
  Refs: FR-7.1–7.10, DESIGN §5.3 (`repos` git columns)

  All of FR-7 via `git2`. Ahead/behind from **local refs only** — never a network fetch against a user repo (FR-7.6). Empty repos handled without error (FR-7.9). Per-repo timeout (FR-7.10).

  **Done when:** Fixtures covering a normal repo, detached HEAD, an empty repo, a dirty tree, and a repo with no upstream all produce correct values with no errors; the timeout is exercised by a test.

- [ ] **M1-4 · Language statistics** · Size: M · Deps: M0-3
  Refs: FR-2.1, FR-2.2

  `tokei` as a library, honoring the prune list. Aggregate into `repo_languages` with percentages; apply the 2% threshold for the primary list while retaining the full breakdown.

  **Done when:** A polyglot fixture produces correct proportions, and vendored directories contribute nothing.

- [ ] **M1-5 · Scan pipeline orchestration** · Size: L · Deps: M1-1, M1-3, M1-4, M0-4
  Refs: DESIGN §6.1, §6.2, §6.4

  `run_scan` on a blocking task; rayon `par_iter` with one repo as the parallelism unit; `RepoAnalysis` sent over `mpsc` to a single writer thread batching transactions. `ScanReporter` trait.

  **Done when:** A 20-repo fixture scan completes, results appear in the DB, and a recording `ScanReporter` observes `repo_done` for each repo **before** `finished` — proving results stream rather than batch.

- [ ] **M1-6 · Cancellation** · Size: M · Deps: M1-5
  Refs: FR-1.9, DESIGN §6.3

  `CancelToken` over `Arc<AtomicBool>`, checked between discovery batches, at each repo's start, and in the writer loop. Cancellation is not an error; partial results persist and the scan is marked `cancelled`.

  **Done when:** Cancelling mid-scan stops within one repo's analysis time, already-scanned repos remain queryable, and the `scans` row reads `cancelled`.

- [ ] **M1-7 · Scan events and progress UI** · Size: M · Deps: M1-5, M0-6
  Refs: DESIGN §12.2, §14.1

  Emit `scan:progress`, `scan:repo_done`, `scan:warning`, `scan:complete`, `scan:error`. Frontend subscribes and invalidates TanStack Query keys (DESIGN §14.1). Global progress indicator with a cancel control.

  **Done when:** The repo list populates progressively during a scan and the progress indicator is visible from every route.

- [ ] **M1-8 · Warning model end to end** · Size: M · Deps: M1-5, M0-7
  Refs: FR-1.10, FR-4.8, DESIGN §4.1, §15

  Recoverable failures become `Warning`s, persisted with the scan and surfaced per repo. `catch_unwind` at the per-repo boundary (DESIGN §15).

  **Done when:** A fixture with an unreadable directory and a deliberately panicking analysis both complete the scan, and both surface as warnings with the repo badged. A repo with warnings must be visually distinguishable from a clean repo — silent degradation is the failure mode this task exists to prevent.

- [ ] **M1-9 · Repo query commands** · Size: M · Deps: M1-5, M0-5
  Refs: DESIGN §12.1

  `scan_start`, `scan_cancel`, `list_repos(RepoFilter)`, `get_repo_detail`. **Filtering and sorting execute in SQL**, not client-side.

  **Done when:** Filters compose correctly and `list_repos` on 50 repos returns in well under 100 ms.

- [ ] **M1-10 · Repo list table** · Size: M · Deps: M1-9, M0-6
  Refs: PRD §6, DESIGN §14.3

  TanStack Table + `@tanstack/react-virtual`. Columns: name, primary language, last commit, dirty flag. Filters and free-text search. Health and category columns are added in M2/M3.

  **Done when:** The table scrolls smoothly and filters/sorts round-trip through the backend.

- [ ] **M1-11 · Onboarding** · Size: S · Deps: M0-9, M1-9
  Refs: PRD §6 (Onboarding), FR-10.4, DESIGN §14.4

  Empty state when no scan root is configured: one-sentence explanation, a plain statement of the network policy, and a folder picker via the dialog plugin.

  **Done when:** A fresh install can reach a completed scan with no configuration beyond choosing a folder.

- [ ] **M1-12 · Incremental re-scan fingerprint** · Size: M · Deps: M1-5
  Refs: DESIGN §6.5

  `blake3(head_sha ‖ Σ sorted(manifest_path ‖ content_hash) ‖ rule_pack_version)`; skip stages 2–3 when unchanged.

  **Done when:** A second scan of an unchanged tree skips analysis for every repo, and touching one lockfile causes exactly that repo to be re-analyzed.

  > **Do not let this optimization reach the matching stage.** Matching always re-runs regardless of fingerprint (DESIGN §6.5). Enforced by test in M2-18.

- [ ] **M1-13 · Read-only invariant test** · Size: M · Deps: M1-5
  Refs: PRD Principle 4, PRD §11, DESIGN §16.4

  Hash every file's content and mtime across a fixture tree, run a full scan, assert nothing changed.

  **Done when:** The test passes and runs in CI. Principle 4 is a promise about the user's source code; it needs a test, not an intention.

- [ ] **M1-14 · Integration test harness** · Size: M · Deps: M1-5
  Refs: DESIGN §16.5

  `tempdir` fixture repo trees (built by a helper that shells out to `git` **in tests only**), in-memory SQLite, recording `ScanReporter`.

  **Done when:** `cargo test -p repo-radar-core` runs a full scan against a generated tree with no Tauri involvement.

---

## M2 — Security core

**This is the milestone the product exists for.** It is also where the highest-risk logic lives. Do not compromise on test coverage here.

> ### Run M2-12 first.
> The compromise/vulnerability split (FR-6.1) is the headline feature, and it rests on an assumption about OSV data that has not been verified. Building the parsers, matcher, scorer, and UI on top of an unverified assumption and discovering it is wrong at the end of the milestone is the single most expensive failure mode available here.

- [ ] **M2-12 · SPIKE: verify OSV malicious-package data** · Size: M · Deps: M0-4 · **Run before the rest of M2**
  Refs: PRD R1, DESIGN §8.2, DESIGN D1

  Download a live snapshot for all four ecosystems. Determine: how many `MAL-` IDs exist per ecosystem; the exact shape of the malicious-code marker in `database_specific`; whether `MAL-` records carry usable `affected[].package` mappings; and whether crates.io and Go have meaningful coverage at all.

  **Done when:** Findings are written into DESIGN §8.2 and D1 is closed or re-scoped. **If an ecosystem's coverage is thin, that is a product decision, not a technical one** — the UI must state it for that ecosystem rather than implying a clean bill of health. Escalate before continuing.

### Version handling and parsers

- [ ] **M2-1 · Version schemes** · Size: M · Deps: M0-3
  Refs: FR-4.6, DESIGN §8.5

  `VersionScheme` trait with SemVer (`semver`), PEP 440 (`pep440_rs`), and Go implementations. Deliberately **not** a shared code path. Unparseable versions produce a `Warning` and mark the dependency unmatchable — never silently skipped.

  **Done when:** Ordering tests pass per scheme, including PEP 440 `rc`/`post`/epoch cases and Go pseudo-versions (DESIGN D5).

- [ ] **M2-2 · Parser trait, registry, and selection** · Size: M · Deps: M0-3
  Refs: FR-4.1, FR-4.2, DESIGN §7.1, §7.2

  `LockfileParser` trait and registry. Selection rule: lockfile → `Exact`; manifest fallback → `Range`.

  **Done when:** A repo with both a lockfile and a manifest yields `Exact`; one with only a manifest yields `Range`.

- [ ] **M2-3 · Name normalization** · Size: S · Deps: M0-3
  Refs: FR-4.5, DESIGN §7.4

  Per-ecosystem rules. Applied identically to dependency names and advisory package names.

  **Done when:** A table-driven test covers each ecosystem's rules — including that **crates.io does not collapse `-`/`_`** (unlike PyPI), since collapsing there would merge genuinely different crates.

- [ ] **M2-4 · npm parsers** · Size: L · Deps: M2-2, M2-3
  Refs: FR-4.1, FR-4.3, FR-4.4, DESIGN §7.3

  `package-lock.json` v1 (nested) and v2/v3 (`packages` map), plus `package.json`. Directness from the root entry's dependency keys, not tree position. Scope from which key it appeared under.

  **Done when:** Fixtures for v1, v2, and v3 produce exact counts and correct direct/transitive and scope flags.

- [ ] **M2-5 · pnpm parser** · Size: M · Deps: M2-2, M2-3
  Refs: DESIGN §7.3

  `pnpm-lock.yaml` via `serde_yaml`. Directness from `importers`. Keys split on the **last** `@` so scoped names parse correctly.

  **Done when:** A workspace fixture with scoped packages parses correctly.

- [ ] **M2-6 · yarn parser (v1), berry decision** · Size: L · Deps: M2-2, M2-3
  Refs: PRD R2, DESIGN §7.3, DESIGN D7

  Hand-written v1 parser. **Timebox berry (v2+).** If it exceeds budget, berry repos fall back to manifest confidence with a visible notice.

  **Done when:** v1 fixtures pass, and berry is either supported or explicitly falling back with a user-visible notice. An honest "unconfirmed" beats a subtly wrong version — do not ship a half-correct berry parser.

- [ ] **M2-7 · Cargo parser** · Size: S · Deps: M2-2, M2-3
  Refs: DESIGN §7.3

  `Cargo.lock` `[[package]]`; scope and directness from `Cargo.toml`'s three dependency tables. Workspace support.

  **Done when:** A workspace fixture parses with correct scopes.

- [ ] **M2-8 · Go parser** · Size: M · Deps: M2-2, M2-3
  Refs: PRD R6, DESIGN §7.3

  **`go.mod`'s `require` block is the dependency set; `go.sum` only confirms versions.** `go.sum` contains hashes for modules not in the final build, often several versions of the same module — using it as the inventory systematically over-reports. `// indirect` marks transitive.

  **Done when:** A fixture whose `go.sum` contains superseded versions yields only the `go.mod` build list.

- [ ] **M2-9 · Python parsers** · Size: L · Deps: M2-2, M2-3
  Refs: FR-4.9, DESIGN §7.3

  `poetry.lock`, `uv.lock`, `Pipfile.lock`, `requirements.txt`, `pyproject.toml`. `requirements.txt` is `Exact` **only if every line is pinned with `==`**; a mixed file is `Range` throughout.

  **Done when:** A mixed pinned/unpinned `requirements.txt` yields `Range` for all entries, not a mixture.

- [ ] **M2-10 · Multi-manifest and monorepo discovery** · Size: M · Deps: M2-4, M2-7, M2-9
  Refs: FR-4.7, DESIGN D3

  Find all manifest roots in a repo; retain `manifest_path` on every dependency; tag repos with more than one root as `monorepo`.

  **Done when:** A monorepo fixture yields dependencies grouped by sub-package, and the repo is tagged.

### OSV

- [ ] **M2-11 · OSV record model, classification, severity** · Size: M · Deps: M2-12
  Refs: FR-6.1, DESIGN §8.1, §8.2, §8.3

  Deserialize the field subset, ignoring unknown fields. `classify()` for Compromise vs Vulnerability, isolated in `has_malicious_marker`. Severity precedence: CVSS_V4 → CVSS_V3 → `database_specific.severity` → `Unscored`. Withdrawn advisories retained but excluded from matching.

  **Done when:** Real records of each kind classify correctly and severity extraction is tested at each precedence level.

- [ ] **M2-13 · Full advisory sync** · Size: L · Deps: M2-11, M0-4
  Refs: FR-5.2, FR-5.8, FR-5.10, DESIGN §8.6

  Stream per-ecosystem zips for **only the ecosystems in use**; incremental extraction and deserialization; ~1,000-record transaction batches; per-ecosystem atomic replacement.

  **Done when:** A real sync of all four ecosystems completes, peak memory stays proportional to batch size rather than advisory count (measured, not assumed), and an interrupted sync leaves the previous snapshot intact.

- [ ] **M2-14 · Incremental advisory sync** · Size: M · Deps: M2-13
  Refs: FR-5.3, DESIGN §8.6

  `modified_id.csv` read until older than the last successful sync; fetch changed IDs via `GET /v1/vulns/{id}` with concurrency 8; fall back to a full zip above 2,000 IDs.

  **Done when:** A second sync fetches only deltas, and the fallback threshold is exercised by a test.

- [ ] **M2-15 · Sync scheduling and resilience** · Size: M · Deps: M2-13
  Refs: FR-5.4, FR-5.5, FR-5.7, DESIGN §13.2, §12.3

  Hourly tick with a 24-hour condition (so a suspended laptop catches up on wake rather than waiting out a dead timer). `sync_lock` so manual and scheduled syncs cannot race. Exponential backoff with jitter, capped at 5 attempts, then logged and surfaced.

  **Done when:** Sync runs on schedule, manual sync cannot overlap scheduled sync, and a network failure surfaces as a notice with the previous snapshot still in use.

- [ ] **M2-16 · Matcher** · Size: L · Deps: M2-1, M2-11, and the parsers
  Refs: FR-6.1, DESIGN §8.4

  Two-phase: SQL narrow on `(ecosystem, package_name)`, then the event-walk decision in Rust. Explicit `versions[]` matched in SQL.

  **Done when:** The full test suite from DESIGN §16.2 passes — hand-written OSV records covering `introduced` with no `fixed`, `last_affected` vs `fixed` boundaries, disjoint ranges, explicit versions, ecosystem-correct event sorting (`1.9.0` vs `1.10.0`), PEP 440 specifics, and withdrawn exclusion. **These are the highest-value tests in the project**; each guards a case that otherwise fails silently.

- [ ] **M2-17 · Health scoring** · Size: M · Deps: M2-16
  Refs: FR-6.2–6.10, DESIGN §9

  Pure function. Compromise **cap** at 39 (not a subtraction). Severity deductions with `×0.4` range and `×0.5` dev/build multipliers. Per-package diminishing returns. Hygiene deductions with a floor of 60.

  **Done when:** Property tests hold: score always in `0..=100`; a confirmed compromise always bands Critical regardless of other inputs; hygiene-only deductions never drop below 60; breakdown amounts sum to `100 - score` when uncapped.

- [ ] **M2-18 · Findings persistence and always-rerun matching** · Size: M · Deps: M2-16, M2-17, M1-12
  Refs: DESIGN §6.5, §6.2 (stage 4)

  Stage 4 runs for every repo on every scan, including fingerprint-unchanged ones.

  **Done when:** A test scans, injects a new advisory affecting an untouched repo, re-scans, and observes that repo's score drop. **This test is Journey B**; without it the product's central value is unverified.

### Security UI

- [ ] **M2-19 · Health tab** · Size: M · Deps: M2-18, M0-6
  Refs: FR-6.9, PRD §6, DESIGN §14.3

  Renders `health_breakdown` JSON **directly** rather than recomputing, so the number shown and the number explained cannot drift. Compromise findings first and visually distinct from vulnerabilities. Every deduction links to its advisory or git fact.

  **Done when:** Every digit of a score is traceable to a named cause in the UI (PRD §11).

- [ ] **M2-20 · Advisories screen and freshness indicator** · Size: M · Deps: M2-15, M2-18
  Refs: FR-5.6, PRD §6, DESIGN §14.3

  Sync status and history, per-ecosystem counts, **Sync now**, and the cross-repo impact view ("which repos does this advisory hit?", using `idx_findings_advisory`). Global freshness indicator escalating past 7 and 30 days.

  **Done when:** The indicator is visible from every route and escalates correctly against a backdated sync timestamp.

- [ ] **M2-21 · Live query fallback** · Size: S · Deps: M2-11
  Refs: FR-5.9, DESIGN §18

  Per-dependency opt-in `POST /v1/query`. Never used during a scan. The UI states that this sends the package name and version externally.

  **Done when:** The action works and is unreachable from any automatic path.

- [ ] **M2-22 · Degraded-state semantics** · Size: S · Deps: M2-20
  Refs: DESIGN §14.4

  When the advisory database has never synced, health is **unknown**, not healthy.

  **Done when:** A fresh install with no sync shows unknown health, visually unambiguous against a healthy score. Showing green for "we haven't checked" would be the worst bug this product could ship.

- [ ] **M2-23 · Health and findings columns in the repo list** · Size: S · Deps: M2-18, M1-10
  Refs: FR-6.1, DESIGN §14.3

  **Done when:** Compromise and vulnerability counts occupy separate columns with separate colors. A combined "issues" column would destroy the distinction the whole health model exists to make.

---

## M3 — Classification

- [x] **M3-1 · Rule pack loading and merge** · Size: M · Deps: M0-3
  Refs: FR-2.6, FR-3.8, DESIGN §10.1, §10.2

  Shipped TOML via `include_str!`; user pack from `<config>/rules/` merged by rule `id` (same id replaces, new id appends). Malformed user pack → startup `Warning`, app continues on the shipped pack. `rule_pack_version` = hash of the merged pack, feeding the scan fingerprint.

  **Done when:** A user override changes behavior without a rebuild; a malformed pack does not prevent startup; editing a rule invalidates cached classifications.

- [x] **M3-2 · Technology detection** · Size: L · Deps: M3-1, M2-10
  Refs: FR-2.3–2.5, DESIGN §10.1

  Dependency signals and marker-file globs. Evidence recorded per detection. Package managers derived from which lockfile is present.

  **Done when:** Detections carry their signals, and marker-only detections render with lower prominence than dependency-confirmed ones (FR-2.4).

- [x] **M3-3 · Categorization engine** · Size: L · Deps: M3-1, M3-2
  Refs: FR-3.1–3.5, DESIGN §10.3

  Weighted accumulation; floor → `Unknown`; frontend+backend both above threshold → `Fullstack`; confidence from margin. Full breakdown serialized to `category_scores`.

  **Done when:** A fixture set of repos of each category classifies correctly, an ambiguous repo yields `Fullstack` rather than an arbitrary pick, and a signal-less repo yields `Unknown` rather than a guess.

- [ ] **M3-4 · Category explainability UI** · Size: M · Deps: M3-3
  Refs: FR-3.6

  Every rule that fired, its signal, its weight, and per-category totals.

  **Done when:** A user can see exactly why a repo was classified as it was. This is the mechanism by which the classifier earns trust — or gets corrected.

- [ ] **M3-5 · Manual category override** · Size: S · Deps: M3-4
  Refs: FR-3.7

  Persists across re-scans; marked as manual; computed value stays visible alongside.

  **Done when:** An override survives a re-scan and the control sits adjacent to the evidence that was wrong.

- [ ] **M3-6 · Dashboard** · Size: L · Deps: M3-3, M2-18
  Refs: PRD §6 (Dashboard)

  Repo count, health distribution histogram, category donut, language bar, stalest repos, worst-health repos, freshness indicator — via Recharts.

  **Done when:** Charts render real data and update on scan completion.

- [ ] **M3-7 · Compromise banner** · Size: S · Deps: M3-6, M2-18
  Refs: FR-6.3, DESIGN §14.3

  **Done when:** The banner renders **only** when compromise findings exist. A banner that is always present is furniture; this one has to mean something when it appears.

---

## M4 — Prompts

- [ ] **M4-1 · Prompt context and rendering** · Size: M · Deps: M3-3, M2-18
  Refs: FR-9.2, DESIGN §11.1

  `PromptContext` serde struct; `minijinja` rendering. Built-ins ship as `.j2` in `assets/prompts/` and run through the same path as user templates — no privileged built-in path.

  **Done when:** A template renders with a fully populated context including advisory freshness.

- [ ] **M4-2 · Built-in templates** · Size: L · Deps: M4-1
  Refs: FR-9.1

  T1 cross-repo similarity (N repos), T2 performance & security opportunities (1 repo, with findings and dependency annotations), T3 code review (1 repo, scoped to whole/dir/files/diff). T2 must instruct the model to distinguish confirmed from speculative issues.

  **Done when:** Each template produces a coherent prompt validated by actually pasting it into an LLM and confirming the response is on-target. These are the product's output; review them as prose, not just as code that runs.

- [ ] **M4-3 · User templates** · Size: S · Deps: M4-1
  Refs: FR-9.2

  Load from `<config>/prompts/*.j2`; document the context object.

  **Done when:** A user template appears in the picker and renders; context documentation is written.

- [ ] **M4-4 · File selection** · Size: L · Deps: M4-1
  Refs: FR-9.3, DESIGN §11.2

  Checkbox tree. Auto-exclude binaries (**content-sniffed, not extension-guessed**), oversized files (256 KB default), gitignored files, and pruned dirs — each shown with its reason rather than silently dropped.

  **Done when:** Exclusions are visible and explained; a binary file with a `.txt` extension is still excluded.

- [ ] **M4-5 · Token estimation** · Size: S · Deps: M4-4
  Refs: FR-9.4

  `chars / 4`, live, against a configurable budget. Labeled as an estimate. Warns on exceed; does not block.

  **Done when:** The estimate updates live and over-budget warns without preventing use.

- [ ] **M4-6 · Preview, copy, export** · Size: M · Deps: M4-2, M4-5
  Refs: FR-9.5–9.7, DESIGN §11.4

  Full preview before any copy or export. Clipboard plugin; export via dialog to a user-chosen path only.

  **Done when:** The full prompt is visible before copying — these may contain proprietary source and the user must see what they are about to send to a third party — and export can never write inside a scanned repo.

- [ ] **M4-7 · Phase 2 seam** · Size: S · Deps: M4-1
  Refs: PRD H1, H3, DESIGN §11.5

  Define `LlmProvider` with no implementation. Verify nothing in generation assumes the clipboard is the destination.

  **Done when:** The trait compiles unused and generation returns a `String` decoupled from delivery. Do **not** ship a disabled provider dropdown (PRD H3).

---

## M5 — Polish and ship

- [ ] **M5-1 · Outdated dependency check (backend)** · Size: L · Deps: M2-10
  Refs: FR-8.1–8.5

  Registry lookups (npm, PyPI JSON, crates.io, Go proxy), batched where supported, rate-limited, descriptive User-Agent. Cached in `outdated_cache` with a 24-hour reuse window. Pre-releases excluded from "latest" unless the installed version is itself a pre-release.

  **Done when:** A repo check returns correct patch/minor/major deltas and the cache is respected.

- [ ] **M5-2 · Outdated check UI** · Size: S · Deps: M5-1
  Refs: FR-8.1, FR-8.6, FR-8.7

  **Done when:** The action is reachable **only** by explicit click, the UI states beforehand that it contacts external registries, and outdated-ness demonstrably does not move the health score. Conflating maintenance lag with security risk would corrupt the signal the score exists to carry.

- [ ] **M5-3 · Settings screen** · Size: M · Deps: M0-9
  Refs: FR-10.1, FR-10.3

  Scan roots, prune list, sync schedule, token budget, theme, **Reset database** (confirmed), **Open data folder**.

  **Done when:** Every setting round-trips and reset clears all derived data.

- [ ] **M5-4 · Empty, error, and degraded states** · Size: M · Deps: M3-6, M2-22
  Refs: DESIGN §14.4, §15

  All seven states from DESIGN §14.4, plus the fatal-error screen offering reset and open-data-folder.

  **Done when:** Each state is reachable in a test build and none is a blank screen or a raw error string.

- [ ] **M5-5 · Performance pass** · Size: M · Deps: M3-6
  Refs: PRD §11, DESIGN §17

  Measure against every PRD §11 target on a real 50-repo tree.

  **Done when:** All targets are met, or a miss is documented with a decision. Do not implement the deferred optimizations in DESIGN §17 unless measurement demands them.

- [ ] **M5-6 · Installers** · Size: L · Deps: M0-8
  Refs: DESIGN §19

  MSI + NSIS, DMG (universal), AppImage + deb.

  **Done when:** Each installs and launches on a clean machine or VM. Signing is deferred (DESIGN §19) — decide before any public distribution.

- [ ] **M5-7 · Documentation** · Size: M · Deps: all

  README with install, first-run, and the network policy stated plainly. Rule pack authoring guide. Prompt context reference.

  **Done when:** Someone else can build, run, and extend the rules without reading the source.

---

## Critical path

```
P-1 → M0-1 → M0-3 → M0-4 → M1-5 → M2-2 → M2-16 → M2-17 → M2-18 → M2-19
                                    ↑
                              M2-1, M2-12
```

Everything else can be scheduled around this. Three notes:

- **M2-12 gates the design**, not just the code. Run it first in M2; its outcome may change what the UI must say.
- **M2-16 and M2-17 are the correctness core.** They are pure functions with no IO by design (DESIGN §3), so they can be built and tested before any parser is finished — using hand-written fixtures.
- **M1-5 unblocks the widest fan-out.** Prioritize it once discovery, git, and languages exist.

---

## Continuous

Not milestone tasks; expectations that hold throughout.

- [ ] `cargo clippy -D warnings` and `cargo fmt` clean on every commit
- [ ] New parser ⇒ new fixture in `crates/core/tests/fixtures/`
- [ ] New scoring or matching rule ⇒ new test in the DESIGN §16.2/16.3 suites
- [ ] `core` never gains a `tauri` dependency (verify with `cargo tree`)
- [ ] No subprocess execution in non-test code — no `npm`, `cargo`, or `git` binaries (DESIGN §18). This is what keeps a scanner from becoming an execution vector when pointed at repositories the user did not write.
- [ ] No new outbound network destination without updating DESIGN §18 and the user-facing network policy

---

## Backlog (explicitly out of scope for v1)

Recorded so they are decisions rather than oversights. From PRD non-goals and DESIGN open questions.

- Java, .NET, PHP, Ruby ecosystems (PRD N7) — the `LockfileParser` trait is the intended extension point
- Secret leak scanning (PRD N4)
- Headless CLI (PRD N2)
- `bun.lockb` binary lockfile (DESIGN D2)
- Per-file language data for prompt selection (DESIGN D6)
- Phase 2: LLM providers, `keyring` API key storage, in-app prompt execution (PRD §9)
- Local model support via an Ollama endpoint (PRD H5)
- Code signing for installers
