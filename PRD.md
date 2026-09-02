# repo-radar — Product Requirements Document

**Status:** Draft v1
**Last updated:** 2026-09-02
**Platforms:** Windows, macOS, Linux
**Stack:** Rust + Tauri 2 + React

---

## 1. Overview & problem statement

Developers accumulate repositories. A `~/repos` or `F:\repos` folder with forty projects in it is normal, and it is opaque: you cannot answer "which of these use React 18?", "which have not been touched in a year?", or — most urgently — "did any of these install a package that was later found to be backdoored?" without opening each one.

**repo-radar** is a local desktop application. You point it at a parent folder; it recursively discovers every git repository beneath it and builds a queryable profile of each:

- **Technologies** — languages, frameworks, build tooling, package managers.
- **Category** — backend, frontend, fullstack, mobile, devops, data/ML, library, CLI, docs.
- **Health** — a transparent 0–100 score dominated by whether the repo's resolved dependency tree contains packages that appear in a published supply-chain compromise or vulnerability advisory.

It also generates structured prompts for LLMs and coding agents, so the analysis can be handed off to a model that reads actual code: find similarities across projects, find performance and security opportunities, and drive code reviews.

Phase 1 runs entirely on your machine. The only network traffic is a scheduled download of a public vulnerability database. Phase 2 adds the option of supplying an LLM API key and running the generated prompts in-app.

### Primary user

A solo developer or small-team lead with 10–50 repositories on one machine, spread across JavaScript/TypeScript, Python, Rust, and Go. They are the only user; there is no multi-user, no server, no account.

---

## 2. Goals and non-goals

### Goals

| # | Goal |
|---|------|
| G1 | Discover every git repo under one or more parent folders, recursively, without configuration. |
| G2 | Identify languages, frameworks, and tooling per repo, with the evidence that led to each conclusion. |
| G3 | Assign each repo a category with a confidence value and an explainable signal list. |
| G4 | Resolve the full transitive dependency tree from lockfiles and match it against published advisories. |
| G5 | Surface *compromised* packages — ones implicated in an actual attack — distinctly from ordinary CVEs. |
| G6 | Keep advisory data fresh via a daily background sync, while remaining fully functional offline. |
| G7 | Generate copy-ready LLM prompts for cross-repo similarity, perf/security review, and code review. |
| G8 | Never modify a scanned repository. |

### Non-goals

| # | Non-goal | Rationale |
|---|----------|-----------|
| N1 | Static application security testing (SAST) of your own source code | Different problem, different engine. repo-radar analyzes *dependencies* and *metadata*. Deep code analysis is delegated to an LLM via generated prompts. |
| N2 | CI/CD gate or headless CLI | This is a desktop exploration tool. A CLI may come later; it is not in scope. |
| N3 | Git client functionality (commit, branch, merge, push) | Read-only by design. Use your existing tools. |
| N4 | Secret/credential leak scanning | Considered and deliberately deferred — it needs its own detection engine and false-positive tuning. |
| N5 | Telemetry, analytics, crash reporting, or any cloud sync | Local-first is a hard product constraint, not a default. |
| N6 | Multi-machine or team aggregation | Single machine, single user. |
| N7 | Java, .NET, PHP, Ruby ecosystems | Out of scope for phase 1. The parser architecture must accommodate them later (see §7.4). |

---

## 3. Principles

These are binding constraints; when a requirement below conflicts with one of these, the principle wins.

1. **Local-first.** All analysis executes on the user's machine. The only outbound network requests in phase 1 are to `storage.googleapis.com` for OSV data, and to package registries when the user explicitly clicks "check for updates". Nothing about the user's code, repo names, or dependency list is ever transmitted.
2. **Transparent scoring.** No opaque numbers. Every health score decomposes into named contributions, and every contribution is clickable through to the advisory, dependency, or git fact that produced it. A score the user cannot audit is a score the user will not trust.
3. **Never block on the network.** The app must be fully usable with the network unplugged. A failed or stale advisory sync produces a visible freshness warning, never a spinner or an error wall.
4. **Read-only against scanned repos.** repo-radar opens git objects for reading and reads manifest/lockfile contents. It never writes a file, never creates a branch, never runs `npm install`, `cargo build`, or any package-manager command inside a scanned repository.
5. **Evidence over assertion.** "This is a backend repo" is useless; "this is a backend repo because it has `express` and `pg` in `package.json`, a `Dockerfile`, and no frontend framework" is actionable. Every classification carries its signals.
6. **Fast enough to leave open.** A warm re-scan should feel instant. Users will not adopt a tool that takes a minute every time they switch to it.

---

## 4. Personas & core journeys

### Journey A — First run

1. User launches repo-radar for the first time. Empty state explains what the app does and asks for a folder.
2. User picks `F:\repos`. App begins scanning; a progress panel streams results as each repo completes rather than waiting for the whole scan.
3. In parallel, the app performs its first advisory sync (four ecosystem downloads) with its own progress indicator. The repo scan does not wait for this.
4. When both finish, the dashboard renders: repo count, category breakdown, language breakdown, health distribution, and — if any exist — a red banner listing compromised packages.

### Journey B — The morning check

1. User opens the app. The last scan's results are already on screen from SQLite; no scan is required to see data.
2. A background incremental re-scan runs, touching only repos whose git HEAD or lockfile hash changed.
3. The advisory sync runs if more than 24 hours have elapsed.
4. If a newly-published advisory now matches an already-known dependency, the affected repos' health scores drop and the dashboard banner updates. This is the core value: *a repo you did not touch became unhealthy because the world changed.*

### Journey C — Preparing a code review

1. User opens a repo's detail page, goes to the **Prompts** tab, selects the "Code review" template.
2. The app pre-fills the repo profile (languages, frameworks, category, dependency summary, open health findings) into the prompt.
3. User optionally selects specific files or directories to embed. A token estimate updates live against a budget.
4. User clicks **Copy** and pastes into Claude, ChatGPT, or their coding agent. Or **Export** to save a `.md` file.

---

## 5. Functional requirements

### FR-1 — Repository discovery

**FR-1.1** The user configures one or more *scan roots* (absolute directory paths). At least one is required for the app to function.

**FR-1.2** The app walks each root recursively using the `ignore` crate, which provides parallel traversal and `.gitignore` awareness.

**FR-1.3** A directory containing a `.git` entry is recorded as a repository. **The walker does not descend into a discovered repository** looking for more repositories, with one exception (FR-1.5). This prevents vendored dependencies containing their own `.git` from being reported as user projects.

**FR-1.4** The following directory names are pruned during traversal and never descended into: `node_modules`, `target`, `.venv`, `venv`, `__pycache__`, `dist`, `build`, `.next`, `.nuxt`, `vendor`, `.cargo`, `.gradle`, `Pods`, `.terraform`. This list is user-editable in settings.

**FR-1.5** Git submodules (declared in a `.gitmodules` file) are recorded as *child* entries associated with their parent repository. They appear in the parent's detail view, not as top-level rows in the repo list. Their dependencies are attributed to the parent.

**FR-1.6** A bare repository (a directory ending in `.git` with no working tree) is detected and listed, but marked as bare; dependency and language analysis are skipped for it since there are no working-tree files.

**FR-1.7** A worktree (`.git` as a *file* containing a `gitdir:` pointer rather than a directory) is handled correctly and resolves to its real git directory.

**FR-1.8** Symbolic links are not followed during traversal, to avoid cycles.

**FR-1.9** Discovery must be interruptible. A **Cancel** control stops an in-flight scan and preserves any results already persisted.

**FR-1.10** Permission errors and unreadable directories are collected into a scan report and shown to the user as warnings; they do not abort the scan.

---

### FR-2 — Technology detection

**FR-2.1 Languages.** Language identification and line counts come from the `tokei` crate used as a library, giving per-language code/comment/blank line counts. Results are stored per repo and normalized into percentages for display.

**FR-2.2** Languages contributing less than 2% of total code lines, and any language found only in pruned directories, are excluded from the primary language list but retained in the full breakdown.

**FR-2.3 Frameworks and tooling.** Framework detection is rule-driven, with rules evaluating two kinds of signal:

- **Dependency signals** — presence of a named package in the resolved dependency inventory (FR-4). Example: `next` → Next.js; `axum` or `actix-web` → Rust web service; `fastapi` → FastAPI; `django` → Django.
- **Marker file signals** — presence of a file matching a glob. Example: `next.config.{js,ts,mjs}`, `Dockerfile`, `docker-compose.y*ml`, `*.tf`, `Chart.yaml`, `.github/workflows/*.y*ml`, `pyproject.toml`, `go.mod`, `tauri.conf.json`.

**FR-2.4** Each detected technology records the signals that fired, and these are shown in the UI on hover or expansion. A technology detected only by a marker file is displayed with lower prominence than one confirmed by a dependency.

**FR-2.5** Detected package managers (npm / pnpm / yarn / bun; pip / poetry / uv / pipenv; cargo; go modules) are reported per repo, derived from which lockfile is present.

**FR-2.6** The rule pack ships as a TOML file inside the app bundle. A user-supplied TOML at `<config-dir>/rules/technologies.toml` is merged over it at startup, letting the user add or override rules without rebuilding. A malformed user rule file produces a visible warning and is ignored rather than crashing the app.

---

### FR-3 — Categorization

**FR-3.1** Each repo receives exactly one **primary category** from this closed set:

`frontend` · `backend` · `fullstack` · `mobile` · `devops` · `data-ml` · `library` · `cli` · `docs` · `unknown`

**FR-3.2** Each repo additionally receives zero or more **secondary tags** drawn from a broader open vocabulary (e.g. `containerized`, `monorepo`, `has-tests`, `desktop`, `game`, `api-client`, `infrastructure`).

**FR-3.3** Categorization is a weighted scoring engine, not a decision tree. Each rule contributes a weight to one or more categories when its signal matches. The category with the highest total wins.

**FR-3.4** Confidence is derived from the margin between the top score and the runner-up, normalized to `high` / `medium` / `low`. A repo whose top two categories are within a narrow margin (e.g. `frontend` 8 vs `backend` 7) is classified `fullstack` if both exceed a threshold, rather than arbitrarily picking one.

**FR-3.5** A repo where no rule fires above a floor threshold is categorized `unknown` rather than guessed at.

**FR-3.6** The repo detail view shows the full scoring breakdown: every rule that fired, its signal, its weight, and the resulting per-category totals. This is the primary mechanism by which the user learns to trust — or correct — the classifier.

**FR-3.7** The user can manually override a repo's category. The override persists across re-scans and is visually marked as manual. The computed category remains visible alongside it.

**FR-3.8** Category rules live in the same shipped-plus-user-override TOML mechanism as FR-2.6, at `<config-dir>/rules/categories.toml`.

---

### FR-4 — Dependency inventory

**FR-4.1** For each repo, the app locates and parses dependency files for the four supported ecosystems:

| Ecosystem | OSV ID | Lockfiles (preferred) | Manifests (fallback) |
|-----------|--------|----------------------|---------------------|
| JavaScript/TypeScript | `npm` | `package-lock.json` (v2/v3), `pnpm-lock.yaml`, `yarn.lock` (v1 and berry), `bun.lockb` | `package.json` |
| Python | `PyPI` | `poetry.lock`, `uv.lock`, `Pipfile.lock`, `requirements.txt` (when fully pinned) | `pyproject.toml`, `requirements.txt` (unpinned), `setup.py` |
| Rust | `crates.io` | `Cargo.lock` | `Cargo.toml` |
| Go | `Go` | `go.sum` | `go.mod` |

**FR-4.2 Resolution confidence.** Every dependency record carries a confidence level:

- **`exact`** — read from a lockfile, giving one concrete resolved version. This is the only level at which advisory matching is authoritative.
- **`range`** — read from a manifest with a version constraint (`^4.17.0`, `>=2,<3`). Matched against advisories by evaluating whether *any* version satisfying the range is affected; results are flagged as *possible, not confirmed*.

**FR-4.3** Dependencies are marked `direct` or `transitive`. Direct dependencies are those named in the manifest; everything else reachable through the lockfile is transitive. This distinction matters because a compromise in a transitive dependency is both more likely and less visible.

**FR-4.4** Dependencies are marked with their scope: `runtime`, `dev`, `build`, `optional`, `peer`. Dev-only dependencies contribute to the health score at a reduced weight (see FR-6.6), since a compromised build-time package is still a real risk on a developer machine but does not ship to production.

**FR-4.5 Normalization.** Package names are normalized per ecosystem before matching, following each ecosystem's own rules — PyPI names are lowercased with runs of `-`, `_`, and `.` collapsed to a single `-`; npm scoped names retain their `@scope/` prefix; Go module paths are kept verbatim including major-version suffixes (`/v2`); crates.io names are lowercased.

**FR-4.6 Version comparison** uses the correct scheme per ecosystem: SemVer for npm and crates.io (`semver` crate), PEP 440 for PyPI (`pep440_rs` crate), and Go's own module version ordering. Using SemVer for PyPI would silently produce wrong results, so this is not a shared code path.

**FR-4.7 Monorepos.** A single repository may contain many manifests (e.g. `packages/*/package.json`, a Cargo workspace, a `pnpm-workspace.yaml`). All are discovered and parsed. Each dependency record retains the path of the manifest it came from, and the repo detail view groups dependencies by sub-package. A repo with more than one manifest root is tagged `monorepo`.

**FR-4.8** A parse failure on one dependency file does not fail the repo. The failure is recorded with the file path and reason, shown as a warning on the repo, and the repo's other dependency files are still processed.

**FR-4.9** `requirements.txt` is treated as a lockfile only when every line is pinned with `==`; a file mixing pins and ranges is treated as a manifest and all its entries get `range` confidence.

---

### FR-5 — Advisory database sync

**FR-5.1 Source.** OSV.dev is the sole vulnerability data source. No GitHub Advisory Database, no NVD, no RSS feeds, no vendor bulletins.

**FR-5.2 Bulk acquisition.** On first sync the app downloads one zip per ecosystem in use:

```
https://storage.googleapis.com/osv-vulnerabilities/npm/all.zip
https://storage.googleapis.com/osv-vulnerabilities/PyPI/all.zip
https://storage.googleapis.com/osv-vulnerabilities/crates.io/all.zip
https://storage.googleapis.com/osv-vulnerabilities/Go/all.zip
```

Only ecosystems actually present in the user's scanned repos are downloaded. The global `all.zip` is never used — it is far larger and mostly irrelevant.

**FR-5.3 Incremental sync.** Subsequent syncs fetch the per-ecosystem change feed:

```
https://storage.googleapis.com/osv-vulnerabilities/<ECOSYSTEM>/modified_id.csv
```

This lists `timestamp,id` pairs in reverse chronological order. The app reads until it reaches an entry older than its last successful sync timestamp, then fetches only those individual advisory records via `GET https://api.osv.dev/v1/vulns/{id}`. If the delta exceeds a threshold (default 2,000 records), the app falls back to a full zip re-download as that is cheaper.

**FR-5.4 Schedule.** A sync runs on app start if more than 24 hours have passed since the last successful sync, and thereafter every 24 hours while the app is running. It runs in the background and never blocks the UI.

**FR-5.5 Manual sync.** A **Sync now** button on the Advisories screen triggers an immediate sync.

**FR-5.6 Freshness indication.** The app persistently displays the age of the advisory data. Beyond 7 days it becomes a visible warning; beyond 30 days, a prominent one. The user must always know how much to trust a green health score.

**FR-5.7 Offline behavior.** A failed sync (no network, DNS failure, HTTP error) is recorded with its reason and surfaced as a dismissible notice. The previous snapshot remains in use and all functionality continues. Failed syncs retry with exponential backoff, not in a tight loop.

**FR-5.8 Storage.** Advisories are normalized into SQLite (§7.5), not left as JSON on disk. Matching is a SQL join, not a linear scan of files.

**FR-5.9 Live query fallback.** For a single dependency, the user may request a live check against `POST https://api.osv.dev/v1/query`. This is opt-in, per-dependency, and exists to answer "is my snapshot missing something?" It is never used during a normal scan.

**FR-5.10** The download is streamed and extracted incrementally; the app must not hold an entire ecosystem's advisory set in memory at once.

---

### FR-6 — Health scoring

**FR-6.1 Two distinct finding classes.** This separation is the core of the feature:

- **Compromise findings.** Advisories whose ID begins with `MAL-`, sourced from the OpenSSF Malicious Packages database that OSV ingests. These describe packages that were actually backdoored, hijacked, or published maliciously — a package that was *part of an attack*. Also included: any advisory whose OSV record is explicitly categorized as malicious code.
- **Vulnerability findings.** Ordinary `CVE-` / `GHSA-` advisories describing bugs with security impact.

These are never merged into one list, one count, or one color. A repo with one compromised package and zero CVEs is in more trouble than a repo with thirty low-severity CVEs, and the UI must make that obvious at a glance.

**FR-6.2 Score range.** Health is an integer 0–100, starting at 100 and reduced by weighted deductions. Bands: **90–100 healthy** · **70–89 attention** · **40–69 at risk** · **0–39 critical**.

**FR-6.3 Compromise deduction.** Any confirmed (`exact` confidence) compromise finding caps the repo's score at **39 (critical)** regardless of other factors, and raises a dashboard-level alert. This is a hard cap, not a subtraction — a compromised package is not something a good score elsewhere should be able to average away.

**FR-6.4 Vulnerability deduction.** Each vulnerability finding deducts based on its CVSS severity, where available from the OSV record:

| Severity | Base deduction |
|----------|---------------|
| Critical | 15 |
| High | 8 |
| Medium | 3 |
| Low | 1 |
| Unscored | 2 |

Deductions from multiple findings on the *same package* are subject to diminishing returns, so one badly-maintained transitive dependency with twelve advisories does not zero out an otherwise sound repo.

**FR-6.5** `range`-confidence findings (FR-4.2) contribute at **40%** of their deduction and are visually marked *unconfirmed*.

**FR-6.6** `dev`- and `build`-scoped findings contribute at **50%** of their deduction.

**FR-6.7 Hygiene deductions.** Small, capped contributions:

- No lockfile for a detected ecosystem: **−5** (this also means dependency data for that ecosystem is `range` confidence, so it compounds appropriately).
- No commit in 365 days: **−5**. No commit in 730 days: **−10** total, not cumulative with the previous.
- Uncommitted changes in the working tree: **−2**.

**FR-6.8** Hygiene deductions in aggregate cannot reduce a score below 60. They signal neglect, not danger, and must never masquerade as a security finding.

**FR-6.9 Explainability.** The repo detail view renders the complete arithmetic: starting value, every deduction with its cause and magnitude, any cap applied, and the final score. Each line links to the underlying advisory or git fact. **This is a hard requirement, not a nice-to-have** — per Principle 2.

**FR-6.10** Score weights live in a config file so they can be tuned without a rebuild, but ship with sensible defaults and are not exposed in the settings UI in phase 1.

---

### FR-7 — Git activity

Read via `git2`, read-only, per repository:

**FR-7.1** Current branch name, or detached-HEAD state with the short SHA.
**FR-7.2** Last commit: SHA, author, committer date, subject line.
**FR-7.3** Commit count in the last 90 days, and total commit count on the current branch.
**FR-7.4** Distinct author count (by email) over the repo's history.
**FR-7.5** Working tree status: clean, or counts of modified / staged / untracked files. Untracked files are counted respecting `.gitignore`.
**FR-7.6** Ahead/behind counts versus the current branch's configured upstream, if one exists. This is computed from local refs only — the app **never** performs a network fetch against a user's repository.
**FR-7.7** Remote origin URL, normalized for display (credentials stripped, `git@host:path` rendered as `host/path`).
**FR-7.8** Local branch count and whether a stash exists.
**FR-7.9** A repository with no commits (freshly `git init`-ed) is handled without error and reported as empty.
**FR-7.10** Git operations on a single repo are bounded by a timeout so that one pathological repository cannot stall a scan indefinitely.

---

### FR-8 — Outdated dependency check (on-demand only)

**FR-8.1** This check **never** runs automatically. Not during a scan, not on a schedule, not on app start. It executes only when the user clicks **Check for updates** on a specific repository.

**FR-8.2** Rationale for the constraint: it requires a network request per distinct package, which for 50 repos is thousands of requests to third-party registries. It is a deliberate, scoped action, not ambient behavior.

**FR-8.3** Latest-version lookups query the relevant registry: npm registry, PyPI JSON API, crates.io API, Go module proxy. Requests are batched where the API supports it, rate-limited, and sent with a descriptive User-Agent.

**FR-8.4** Each dependency is reported as up-to-date, or behind by patch / minor / major, with the latest stable version. Pre-release versions are excluded from "latest" unless the installed version is itself a pre-release.

**FR-8.5** Results are cached in SQLite with a timestamp and displayed with their age. A repeat check within 24 hours reuses the cache unless the user forces a refresh.

**FR-8.6** Outdated-ness does **not** affect the health score. Being three minor versions behind is a maintenance fact, not a security fact, and conflating the two would corrupt the signal the score exists to carry.

**FR-8.7** The UI states plainly, before the user clicks, that this action contacts external package registries.

---

### FR-9 — AI prompt generator

**FR-9.1** Three built-in templates, corresponding directly to the three stated use cases:

**T1 — Cross-repo similarity.** Input: two or more selected repositories. The prompt embeds each repo's profile (languages with proportions, frameworks, category and its signals, direct dependency list, directory structure to a bounded depth, notable config files) and asks the model to identify shared patterns, duplicated logic, candidates for extraction into a shared library, and inconsistencies in how the same problem was solved differently across projects.

**T2 — Performance & security opportunities.** Input: one repository. The prompt embeds the repo profile, the full dependency inventory with direct/transitive and scope annotations, all open health findings with their advisory details, git activity, and the largest/most-central source files. It asks for concrete, prioritized, actionable improvements — explicitly instructing the model to distinguish confirmed issues from speculative ones.

**T3 — Code review.** Input: one repository, plus a user-chosen scope — the whole repo, a directory, a specific file set, or the current uncommitted diff. The prompt embeds the repo profile as context, then the selected code, and asks for a review covering correctness, error handling, security, and maintainability.

**FR-9.2 Custom templates.** Templates are `minijinja` files. User templates are read from `<config-dir>/prompts/*.j2` and appear in the template picker alongside the built-ins. The full context object (repo profile, dependencies, findings, git data) is documented and available to any template.

**FR-9.3 File embedding.** For templates that embed source, the user selects files or directories via a tree with checkboxes. Binary files, files exceeding a size limit, and gitignored files are excluded automatically and shown as such.

**FR-9.4 Token budget.** A live token estimate is displayed as the user adjusts selections, against a configurable budget (default 100,000). Exceeding the budget is a visible warning, not a hard block — the user may know better. The estimate is a heuristic approximation (character-based, ~4 chars/token) and is labeled as an estimate, not a precise count.

**FR-9.5 Output.** Two actions: **Copy to clipboard** (via the Tauri clipboard plugin) and **Export to `.md`** (via the dialog plugin, user chooses the destination). Exported files are written where the user says — never inside a scanned repository.

**FR-9.6** The rendered prompt is shown in a scrollable preview before copying, so the user can see exactly what they are about to send to a third party. Given that these prompts may contain proprietary source code, this transparency is required, not optional.

**FR-9.7** Generated prompts are not persisted by default. The user may explicitly save one.

---

### FR-10 — Settings & persistence

**FR-10.1** Configurable: scan roots (add/remove/reorder), additional prune directory names, additional file-extension exclusions, advisory sync schedule (daily / manual only), token budget default, theme (light / dark / system).

**FR-10.2** Config is stored via the Tauri store plugin in the OS-appropriate config directory. The SQLite database lives in the OS-appropriate data directory. Neither is written into a scanned repo, and neither is written to the repo-radar source tree at runtime.

**FR-10.3** Settings include a **Reset database** action (with confirmation) that clears all scan results and advisory data, and an **Open data folder** action.

**FR-10.4** The app functions on first launch with zero configuration beyond choosing a scan root.

---

## 6. Screens

| Screen | Contents |
|--------|----------|
| **Onboarding** | Shown when no scan root is configured. Explains the app in a sentence, states the local-first/network policy plainly, and offers a folder picker. |
| **Dashboard** | Aggregate view. Compromise alert banner (only rendered if compromise findings exist — it must mean something when it appears). Repo count, health distribution histogram, category donut, language bar chart, advisory freshness indicator, stalest repos, worst-health repos. |
| **Repo list** | Virtualized sortable/filterable table: name, category, primary language, health score with band color, finding counts (compromise / vuln shown separately), last commit, dirty flag. Filters on category, language, health band, and has-findings. Free-text search on name and path. |
| **Repo detail** | Tabbed. **Overview** — path, git activity, category with signal breakdown, technology list. **Tech** — language chart, frameworks with evidence, package managers, notable files. **Dependencies** — grouped by ecosystem and sub-package, filterable by direct/transitive and scope, with the on-demand update check. **Health** — full score arithmetic, compromise findings first and visually distinct, then vulnerability findings, each expandable to the advisory detail. **Prompts** — template picker, scope selection, token meter, preview, copy/export. |
| **Advisories** | Sync status and history, per-ecosystem advisory counts and last-updated times, **Sync now**, and a cross-repo findings view answering "which of my repos are affected by this advisory?" |
| **Settings** | Per FR-10. |

Global: a scan progress indicator visible from any screen while a scan is running, and a persistent advisory-freshness indicator.

---

## 7. Architecture

### 7.1 Process model

Standard Tauri two-process split. The Rust core owns all filesystem access, git access, parsing, network I/O, SQLite, and scoring. The React frontend is a pure view layer: it renders state and issues commands, and contains no analysis logic.

### 7.2 Command and event boundary

Frontend → backend via Tauri commands (`scan_start`, `scan_cancel`, `list_repos`, `get_repo_detail`, `sync_advisories`, `check_outdated`, `render_prompt`, `get_settings`, `set_settings`).

Backend → frontend via Tauri events, because scans are long-running and results must stream: `scan:progress` (repos discovered/completed), `scan:repo_done` (one repo's summary, so the UI populates incrementally), `scan:complete`, `scan:error`, `sync:progress`, `sync:complete`.

Long operations must never be a single blocking command invocation. The UI must be responsive and cancellable throughout.

### 7.3 Scan pipeline

```
discover (ignore, parallel walk, prune)
  → for each repo, in parallel (rayon):
        git metadata (git2)
        language stats (tokei)
        manifest + lockfile discovery
        dependency parsing → normalized (ecosystem, name, version, confidence, scope, direct?)
        technology rules
  → categorize (weighted scoring over deps + markers + languages)
  → match dependencies against advisory tables (SQL join)
  → score health
  → persist to SQLite, emit scan:repo_done
```

The parallelism unit is the repository. With 10–50 repos this saturates available cores without needing finer-grained work stealing.

### 7.4 Parser extensibility

Each dependency file format implements a common trait:

```rust
trait LockfileParser {
    fn ecosystem(&self) -> Ecosystem;
    fn matches(&self, path: &Path) -> bool;
    fn parse(&self, content: &str, path: &Path) -> Result<Vec<Dependency>, ParseError>;
}
```

Parsers are registered in a list. Adding Java/.NET/PHP/Ruby later (N7) means adding parsers and an ecosystem identifier — no changes to the pipeline, matcher, or scorer. This is the intended extension point.

### 7.5 SQLite schema (sketch)

```
scan_roots        (id, path, enabled, added_at)
repos             (id, root_id, path, name, is_bare, parent_repo_id,
                   head_sha, branch, last_commit_at, commits_90d,
                   author_count, dirty_count, ahead, behind, remote_url,
                   category, category_confidence, category_manual,
                   health_score, health_band, last_scanned_at)
repo_languages    (repo_id, language, code_lines, percentage)
repo_technologies (repo_id, tech, kind, evidence_json)
manifests         (id, repo_id, path, ecosystem, kind, content_hash)
dependencies      (id, repo_id, manifest_id, ecosystem, name, version,
                   confidence, scope, is_direct)
advisories        (id, source_id, ecosystem, kind, summary, details,
                   severity, cvss_score, published, modified, aliases_json)
affected_ranges   (id, advisory_id, ecosystem, package_name,
                   introduced, fixed, last_affected, event_type)
findings          (id, repo_id, dependency_id, advisory_id, kind,
                   confidence, deduction, suppressed)
outdated_cache    (ecosystem, name, latest_version, checked_at)
scans             (id, started_at, finished_at, repo_count, status, warnings_json)
sync_log          (id, ecosystem, started_at, finished_at, mode, count, status, error)
```

Indexes on `affected_ranges(ecosystem, package_name)` and `dependencies(ecosystem, name)` make matching a fast join rather than a scan. This is the single most performance-critical index pair in the schema.

### 7.6 Incremental re-scan

A repo is re-analyzed only if its git HEAD SHA changed, any manifest/lockfile content hash changed, or it has never been scanned. Otherwise its stored analysis is reused.

**Advisory matching is always re-run**, even for unchanged repos — because the advisory database changes independently of the code. This is precisely Journey B, and getting it wrong would defeat the product's main purpose.

### 7.7 Concurrency and safety

Bounded thread pool for repo analysis (`rayon`). Bounded concurrency for network requests. All git and filesystem operations open handles read-only. No package-manager subprocess is ever spawned. Panics in a single repo's analysis are caught at the repo boundary and converted to a recorded warning, so one malformed lockfile cannot take down a scan.

---

## 8. Tech stack

### Prerequisite

**Rust is not currently installed on the development machine.** Before any code can be built:

- `rustup` with the stable toolchain
- Windows: **Microsoft C++ Build Tools** (MSVC) — required by `rusqlite`'s bundled SQLite, `git2`'s libgit2, and Tauri itself
- Linux: `webkit2gtk` and related Tauri system dependencies
- macOS: Xcode Command Line Tools
- Node.js 20+ (24.11.1 present) and a package manager for the frontend

### Frontend

| Concern | Choice | Note |
|---------|--------|------|
| Framework | React 19 + TypeScript | |
| Build | Vite | Tauri's default and best-supported pairing |
| Styling | Tailwind CSS | |
| Components | shadcn/ui | Copy-in components; no runtime dependency to audit |
| Server state | TanStack Query | Wraps Tauri command invocations |
| Tables | TanStack Table + virtualization | Required for the repo list at scale |
| Charts | Recharts | Dashboard breakdowns |
| Routing | React Router | |

### Backend (Rust)

| Crate | Purpose |
|-------|---------|
| `tauri` 2.11 + plugins (`dialog`, `fs`, `store`, `opener`, `clipboard-manager`) | Shell, folder picker, config, clipboard |
| `ignore` | Parallel filesystem walk with gitignore support |
| `git2` | Git metadata. Chosen over `gix` for a stable, complete API covering exactly what FR-7 needs; `gix` is pure-Rust and avoids a C toolchain, but its API churns and its higher-level conveniences are less settled. Isolated behind an internal trait so it can be swapped. |
| `tokei` | Language detection and line counting |
| `rayon` | Data-parallel repo analysis |
| `rusqlite` (`bundled`) | Embedded SQLite, no system dependency |
| `reqwest` (`rustls-tls`) | HTTPS for OSV and registries; rustls avoids OpenSSL build pain across the three platforms |
| `zip` | Streaming extraction of OSV bulk archives |
| `serde`, `serde_json`, `toml`, `serde_yaml` | Manifests, lockfiles, rule packs. `serde_yaml` is needed for `pnpm-lock.yaml`. |
| `semver` | npm and crates.io version ranges |
| `pep440_rs` | PyPI version ordering — deliberately not SemVer |
| `minijinja` | Prompt templating |
| `tracing` | Structured local logging |
| `thiserror` / `anyhow` | Error handling |

Reserved for phase 2: `keyring` (OS credential store for API keys).

---

## 9. Phase 2 design hooks

Phase 2 adds "paste an API key, run the prompt in-app". It is **not** built in phase 1, but phase 1 must not preclude it:

**H1** — Prompt generation is already separated from prompt *delivery*. An `LlmProvider` trait (`async fn complete(&self, prompt: &str) -> Result<Stream<String>>`) is the only new abstraction required; the existing generator feeds it unchanged.

**H2** — API keys will be stored in the OS credential store via `keyring` (Windows Credential Manager, macOS Keychain, Linux Secret Service). **Never** in SQLite, never in the config file, never in plaintext.

**H3** — The prompt panel is laid out with room for a provider selector and a results pane. Phase 1 ships without them rather than shipping them disabled.

**H4** — When phase 2 lands, the local-first guarantee changes materially: source code will leave the machine. This must be an explicit, per-provider, informed opt-in with a clear indication of what is being sent — not a checkbox buried in settings.

**H5** — Local model support (e.g. an Ollama endpoint on localhost) is the natural first `LlmProvider` implementation, since it preserves the offline guarantee entirely. Considered for phase 2, not committed.

---

## 10. Milestones

| # | Milestone | Contents |
|---|-----------|----------|
| **M0** | Scaffold | Tauri 2 + React + TS + Vite project, Tailwind/shadcn, SQLite with migrations, logging, app shell and navigation, CI building on all three platforms. |
| **M1** | Discovery | FR-1 discovery, FR-7 git metadata, FR-2.1–2.2 language stats. Repo list renders real data with streaming progress. |
| **M2** | Security core | FR-4 dependency parsing, FR-5 OSV sync, FR-6 health scoring, Advisories screen, Health tab. *The product is useful at the end of this milestone.* |
| **M3** | Classification | FR-2.3–2.6 technology rules, FR-3 categorization with explainability and override, Dashboard with charts. |
| **M4** | Prompts | FR-9 templates, file selection, token estimation, copy/export. |
| **M5** | Polish & ship | FR-8 outdated check, FR-10 settings, empty/error states, performance pass, installers (MSI/NSIS, DMG, AppImage/deb). |

M2 is the milestone that must not be compromised on quality; it carries the feature the product exists for.

---

## 11. Success metrics

| Metric | Target |
|--------|--------|
| Cold scan, 50 repos | < 60 s |
| Warm incremental re-scan, 50 repos | < 10 s |
| First advisory sync (4 ecosystems) | < 5 min on a typical connection |
| Incremental advisory sync | < 30 s |
| UI responsiveness during scan | No frame longer than 100 ms; cancel responds within 1 s |
| Memory, 50 repos | < 500 MB peak |
| Writes into scanned repos | **Zero** — verified by test |
| Offline functionality | All features except sync and outdated-check work with the network disabled |
| Score explainability | 100% of deductions traceable to a named cause in the UI |
| False-positive compromise findings | Zero tolerance; a wrong red banner destroys trust in the whole product |

---

## 12. Open questions & risks

| # | Item | Impact | Approach |
|---|------|--------|----------|
| R1 | **`MAL-` coverage in OSV per ecosystem is unverified.** FR-6.1 depends on OSV carrying OpenSSF Malicious Packages advisories with usable ecosystem/package mapping. Coverage is believed strong for npm and PyPI, less certain for crates.io and Go. | High — this is the headline feature | Verify against a real snapshot during M2, before building the UI on top of it. If coverage is thin for an ecosystem, say so in the UI rather than implying a clean bill of health. |
| R2 | **`yarn.lock` berry (v2+) format** differs substantially from v1 and is YAML-ish but not standard YAML. | Medium | Budget separate effort. If it proves costly, ship v1 first and treat berry repos as manifest-confidence with a visible notice. |
| R3 | **Monorepo dependency attribution** — whether findings should roll up to the repo or stay scoped to the sub-package. | Medium | FR-4.7 keeps manifest paths on every record, so both views are derivable. Default to repo-level rollup with sub-package drill-down. |
| R4 | **OSV archive size growth** may make full downloads slow over time. | Low–Medium | `modified_id.csv` incremental sync (FR-5.3) is the mitigation. Monitor sizes during M2. |
| R5 | **Single-source risk.** OSV is the only vulnerability source by design. An OSV outage, schema change, or coverage gap has no fallback. | Medium | Accepted trade-off for simplicity. The parser/matcher is source-agnostic internally, so a second source could be added later without restructuring. |
| R6 | **Go `go.sum` lists hashes for modules not actually in the build**, including multiple versions of the same module. Naive parsing over-reports. | Medium | Use `go.mod` to determine the actual build list and treat `go.sum` as version confirmation, not as the dependency set. |
| R7 | **Version-range matching for `range`-confidence dependencies** is inherently approximate. | Medium | FR-4.2 and FR-6.5 already mark these as unconfirmed and de-weight them. Never present a range match with the same visual weight as an exact one. |
| R8 | **`git2`/libgit2 build friction on Windows** requires MSVC build tools. | Low | Documented as a prerequisite (§8). The trait isolation makes a `gix` swap possible if it becomes a real problem. |
| R9 | **Repos on network drives or with very large histories** may make FR-7.3/7.4 slow. | Low | FR-7.10 timeout, plus consider capping history walks at a commit count. |
