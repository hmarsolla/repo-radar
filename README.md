# repo-radar

Local-first repository intelligence and dependency-security scanning, packaged
as a desktop app (Tauri 2 + React). Point it at the folders where you keep git
repositories; it inventories them, classifies them, extracts git health
signals, and cross-references their dependencies against the
[OSV](https://osv.dev) advisory database — separating **confirmed compromises**
(backdoored / malicious packages) from ordinary vulnerabilities.

See [PRD.md](PRD.md) for requirements, [DESIGN.md](DESIGN.md) for the technical
design, and [TASKS.md](TASKS.md) for the build plan.

## Network policy

repo-radar is local-first. The **only** outbound traffic is:

- **OSV advisory data** — bulk downloads from
  `osv-vulnerabilities.storage.googleapis.com` / `api.osv.dev`. The whole
  ecosystem database is downloaded and matched **locally**; your dependency
  list never leaves the machine.
- **Package registries** — `registry.npmjs.org`, `pypi.org`, `crates.io`,
  `proxy.golang.org`, and **only** when you explicitly click **Check for
  updates** on a repository (FR-8). Those requests carry package names only.
- **`api.osv.dev` single-package query** — only if you use the opt-in
  *Check a single package live* box on the Advisories screen, which says so.

Nothing about your source code is ever transmitted. There is no telemetry.
Git access is entirely in-process (libgit2) — repo-radar never shells out to
`git`, `npm`, or `cargo`, so pointing it at a repository you did not write is
not an execution risk.

## Installing

Download the installer for your platform from the releases page and run it:

| Platform | Artifact |
|---|---|
| Windows | `.msi` or `.exe` (NSIS) |
| macOS | `.dmg` (universal) |
| Linux | `.AppImage`, or `.deb` on Debian/Ubuntu |

Builds are currently **unsigned**, so Windows SmartScreen and macOS Gatekeeper
will warn on first launch (Windows: *More info → Run anyway*; macOS:
right-click → *Open*). To build your own, see [Development](#development).

## First run

1. Launch repo-radar. With nothing configured yet it shows a one-screen
   explainer and a **Choose a folder** button.
2. Pick a directory that contains git repositories (it can be nested — the
   walker stops at each `.git` and skips `node_modules`, `target`, `.venv`,
   and friends). The network policy above is restated on this screen.
3. The scan starts automatically. Repositories stream into the list as they
   finish; each gets a language breakdown, git health, and a category.
4. Open **Advisories → Sync now** to download the OSV database for the
   ecosystems your repos use. Until this runs, health is shown as
   **unknown** — *not* healthy.
5. After the sync, every repo has a health score with a fully itemized
   breakdown on its **Health** tab. Confirmed compromises are always ranked
   Critical and shown separately from vulnerabilities.

Everything repo-radar stores is derived — the database lives in your OS data
directory and can be rebuilt at any time from **Settings → Reset database**
followed by a re-scan. Nothing is ever written inside a scanned repository.

## Configuring and extending

- **Settings** — scan roots (add / remove / reorder / disable), extra prune
  directories, excluded file extensions for the prompt picker, advisory sync
  schedule (daily / manual), prompt token budget, theme, **Reset database**,
  **Open data folder**.
- **Custom classification rules** — layer your own category and technology
  rules on top of the built-ins without rebuilding:
  [docs/rule-packs.md](docs/rule-packs.md).
- **Custom prompt templates** — drop `*.j2` files in the config `prompts/`
  folder; the context they render against is documented in
  [docs/prompt-context.md](docs/prompt-context.md).

The config directory (which holds `rules/`, `prompts/`, and `settings.json`)
is:

| OS | Path |
|---|---|
| Windows | `%APPDATA%\dev.repo-radar\` |
| macOS | `~/Library/Application Support/dev.repo-radar/` |
| Linux | `~/.config/dev.repo-radar/` |

## Repository layout

```
crates/core/   repo-radar-core — all analysis logic, no Tauri dependency
src-tauri/     the desktop shell: state, commands, events
src/           React frontend (bindings.ts is generated — do not edit)
```

## Prerequisites

- **Rust** (stable) via [rustup](https://rustup.rs), with the platform C
  toolchain: MSVC Build Tools on Windows, `build-essential` on Linux, Xcode
  CLT on macOS.
- **Node 24+**.
- **Linux only** — Tauri system libraries:
  `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libsoup-3.0-dev`.

## Development

```bash
npm install
npm run tauri dev      # launches the app with hot reload
```

Other useful scripts:

| Command | What it does |
|---|---|
| `npm run bindings` | Regenerate `src/bindings.ts` from the Rust command/event definitions |
| `npm run lint` / `npm run typecheck` | ESLint / `tsc --noEmit` |
| `cargo test --workspace` | Rust unit + integration tests |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lints |
| `npm run tauri build` | Production bundle (installers land in `target/release/bundle/`) |

`src/bindings.ts` is generated and committed; CI fails if it drifts from the
Rust types.

## License

MIT OR Apache-2.0.
