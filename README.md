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

- **OSV advisory data** — bulk downloads from `storage.googleapis.com` /
  `api.osv.dev`. The whole ecosystem database is downloaded and matched
  **locally**; your dependency list never leaves the machine.
- **Package registries** — npm / PyPI / crates.io / the Go module proxy, and
  **only** when you explicitly run an "check for outdated dependencies" action.

Nothing about your source code is ever transmitted. There is no telemetry.
Git access is entirely in-process (libgit2) — repo-radar never shells out to
`git`, `npm`, or `cargo`, so pointing it at a repository you did not write is
not an execution risk.

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
| `npm run tauri build` | Production bundle |

`src/bindings.ts` is generated and committed; CI fails if it drifts from the
Rust types.

## License

MIT OR Apache-2.0.
