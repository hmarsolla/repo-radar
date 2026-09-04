# Release checklist — M5-5 (performance) & M5-6 (installers)

These two tasks are verification, not code. Run them on the target hardware
and record the results. Nothing here should require source changes; if a
target is missed, the outcome is a *documented decision*, per
[TASKS.md](../TASKS.md) M5-5.

---

## M5-5 — Performance pass (PRD §11, DESIGN §17)

### Set up the measurement tree

- [ ] Assemble a **real 50-repo tree** (not fixtures): a directory that holds
      ~50 actual git repositories across the four ecosystems (npm, PyPI,
      crates.io, Go), including at least one large monorepo and a few repos
      with `node_modules` / `target` present on disk (these exercise the
      prune list). Note total size on disk.
- [ ] Machine spec for the record: CPU, RAM, disk type (SSD/NVMe), OS.
- [ ] Build in release: `npm run tauri build`, then run the bundled app (do
      **not** measure `tauri dev` — the debug core is far slower).

### Targets

| # | Metric | Target | How to measure | Result |
|---|---|---|---|---|
| 1 | Cold scan, 50 repos | **< 60 s** | Fresh DB (Settings → Reset database), click **Scan now**, time from click to the scan-complete state. Cross-check against `scans.finished_at − started_at` and the `"advisory ecosystem synced"` / scan `tracing` lines in `<data>/logs/`. | |
| 2 | Warm incremental re-scan, 50 repos | **< 10 s** | Immediately re-run the scan with nothing changed. Every repo should hit the fingerprint skip; only git HEAD reads + manifest hashing + the always-run match step should run. | |
| 3 | Targeted re-scan | (sanity) | `touch` one lockfile, re-scan: exactly that repo re-analyzes, still < 10 s total. | |
| 4 | First advisory sync (4 ecosystems) | **< 5 min** on a typical connection | Advisories → **Full re-sync** from empty. Record wall time and peak memory (see #6). | |
| 5 | Incremental advisory sync | **< 30 s** | Advisories → **Sync now** a few minutes later. | |
| 6 | Memory, 50 repos | **< 500 MB peak** | Watch RSS of the app process through a full cold scan **and** a full advisory sync (the sync's streaming ingest is the memory-sensitive path — FR-5.10). Windows: Task Manager / `Get-Process`. macOS/Linux: Activity Monitor / `/usr/bin/time -l` / `htop`. | |
| 7 | UI responsiveness during scan | No frame > **100 ms**; **cancel responds < 1 s** | With the scan running, navigate between routes and scroll the repo table — it must stay smooth (virtualized). Click **Cancel**; the scan must stop within ~1 s and already-scanned repos stay queryable. Optionally capture a DevTools performance trace and check for long tasks on the main thread. | |
| 8 | Offline functionality | All features except sync + outdated-check work network-disabled | Disable networking. Confirm: scan runs; classification, health (against the last synced DB), dashboard, prompts all work; **Sync now** and **Check for updates** fail with a clear notice and the previous snapshot/results stay in use. | |
| 9 | Writes into scanned repos | **Zero** | Already enforced by the M1-13 read-only invariant test in `cargo test --workspace`. Re-confirm it is green in CI for this build. | |
| 10 | Score explainability | 100% of deductions traceable | Spot-check 5 repos with findings: every number on the **Health** tab has a named cause (advisory id or git fact). This is structural (the UI renders stored `health_breakdown`), so this is a sanity check, not a measurement. | |

### If a target is missed

Record it here with a decision. Do **not** reach for the DESIGN §17 deferred
optimizations (within-repo parallelism, scan daemon, mmap'd advisory index)
unless a specific measurement demands it.

- Target: …
- Measured: …
- Decision: (accept / mitigate / optimize) …

### Write results back

- [ ] Update DESIGN §17 (or add a short "measured" note) with the real
      numbers and the machine spec.
- [ ] Tick M5-5 in [TASKS.md](../TASKS.md).

---

## M5-6 — Installers (DESIGN §19)

`tauri.conf.json` already sets `bundle.targets: "all"`, so `npm run tauri
build` produces every installer the host platform can make. Bundles land in
`src-tauri/target/release/bundle/`.

### Build

| Platform | Command / host | Produces |
|---|---|---|
| Windows | `npm run tauri build` on Windows | `.msi` (WiX) **and** `.exe` (NSIS) under `bundle/msi/` and `bundle/nsis/` |
| macOS | `npm run tauri build` on macOS (add `--target universal-apple-darwin` for a universal binary; install both `aarch64-apple-darwin` and `x86_64-apple-darwin` rustup targets first) | `.app` and `.dmg` under `bundle/dmg/` and `bundle/macos/` |
| Linux | `npm run tauri build` on Debian/Ubuntu | `.AppImage` under `bundle/appimage/` and `.deb` under `bundle/deb/` |

- [ ] All three platforms build without error (CI's `tauri build` matrix
      already covers this — confirm the run is green for the release commit).
- [ ] Version in `src-tauri/tauri.conf.json` and `package.json` matches the
      release tag.

### Verify each artifact on a clean machine or VM

For every installer, on a machine that has **never** run repo-radar:

- [ ] **Windows `.msi`** — installs; app launches; SmartScreen warning is
      expected (unsigned) — *More info → Run anyway*. Uninstall via
      Add/Remove Programs leaves no app data behind beyond `%APPDATA%\dev.repo-radar\`.
- [ ] **Windows `.exe` (NSIS)** — same checks; installs per-user without admin.
- [ ] **macOS `.dmg`** — mounts; drag-to-Applications works; first launch
      needs right-click → **Open** (unsigned, Gatekeeper). Verify it runs on
      both Apple Silicon and Intel if a universal build was made.
- [ ] **Linux `.AppImage`** — `chmod +x` then run directly on a clean distro
      (test at least one that is *not* your build host); window opens.
- [ ] **Linux `.deb`** — `sudo apt install ./repo-radar_*.deb` pulls its
      webkit2gtk dependency; app launches from the menu.

### First-run smoke test (each platform)

- [ ] Onboarding screen appears (no scan roots configured).
- [ ] Choosing a folder starts a scan; repos appear.
- [ ] Data lands in the OS **data** dir, config in the OS **config** dir;
      nothing is written into the scanned folder or next to the executable
      (FR-10.2).
- [ ] **Advisories → Sync now** downloads and completes.

### Signing

Deferred past M5 (DESIGN §19). Decide before any public distribution:

- [ ] Windows: Authenticode certificate (EV recommended to skip SmartScreen
      reputation build-up).
- [ ] macOS: Developer ID Application cert + notarization + stapling.
- [ ] Record the decision (sign now / ship unsigned with the warnings
      documented in the README) here.

### Write results back

- [ ] Note any per-platform quirks in DESIGN §19.
- [ ] Tick M5-6 in [TASKS.md](../TASKS.md).
