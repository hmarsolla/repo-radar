# Rule pack authoring guide (FR-2.6, FR-3.8)

repo-radar classifies each repository into a **category** (Frontend, Backend,
Fullstack, …) and detects the **technologies** it uses. Both are driven by
TOML *rule packs*. A shipped pack is built into the binary; you can layer your
own on top without rebuilding.

- **Category rules** — [`crates/core/assets/categories.toml`](../crates/core/assets/categories.toml)
- **Technology rules** — [`crates/core/assets/technologies.toml`](../crates/core/assets/technologies.toml)

Open either file to see the full shipped set; the examples below are the same
shape.

## Where your packs go

Drop one or more `*.toml` files into the `rules/` folder inside repo-radar's
**config** directory:

| OS | Path |
|---|---|
| Windows | `%APPDATA%\dev.repo-radar\rules\` |
| macOS | `~/Library/Application Support/dev.repo-radar/rules/` |
| Linux | `~/.config/dev.repo-radar/rules/` |

The folder does not exist until you create it — that is the normal first-run
state. Every `*.toml` in it is loaded, sorted by file name, and merged into
the shipped pack. A single file may contain category rules, technology rules,
or both.

## How merging works

- Merge is **by `id`**. A rule whose `id` matches a shipped (or earlier-loaded)
  rule **replaces it in place**; a new `id` is **appended**. Shipped ordering
  is preserved.
- The `[settings]` block, if you include one, **replaces** the shipped settings
  wholesale — copy all four keys, not just the one you want to change.
- Editing any pack changes `rule_pack_version` (a hash of the merged text),
  which invalidates every repo's cached classification. The next scan
  re-classifies; you do **not** need to rebuild the app.
- A pack that fails to parse does **not** stop startup: it is skipped and
  surfaced as a startup warning (the shield/bell area), and the rest of the
  packs still load.

To **disable** a shipped rule, override its `id` with harmless content, e.g.
`weights = { }` for a category rule.

## Category rules

```toml
[settings]
floor = 3               # top category score below this → "Unknown" (no guess)
fullstack_threshold = 4 # Frontend AND Backend both ≥ this …
margin = 2              # … and within this of each other → "Fullstack"

[[rule]]
id = "internal-rpc-framework"     # unique; re-use a shipped id to override it
weights = { Backend = 5 }         # added to each named category when the rule fires
any_dependency = ["@acme/rpc", "acme-rpc"]
```

A rule **fires** when *any one* of its populated signals matches (signals are
OR-ed). When it fires, every entry in `weights` is added to that category's
running total. After all rules run:

- Highest-scoring category wins. If its score is below `floor` → **Unknown**
  (FR-3.5 — admitting ignorance beats guessing).
- If Frontend and Backend are both ≥ `fullstack_threshold` and within `margin`
  of each other → **Fullstack**.
- Confidence is the gap to the runner-up: `> 5` High, `> 2` Medium, else Low.

Every rule that fired, its signal, and its weight are shown in the repo's
**Category** panel, so an override is always traceable.

### Signals

| Key | Type | Fires when |
|---|---|---|
| `any_dependency` | list of names | any listed package is a resolved dependency |
| `all_dependencies` | list of names | **every** listed package is present |
| `any_file` | list of globs | any matching file exists in the repo |
| `any_language` | list of `{ language, min_percentage? }` | a language is present at ≥ `min_percentage` (default 0), case-insensitive |
| `predicate` | string | a named built-in returns true (closed set, below) |

**Dependency names** are matched after per-ecosystem normalization, the same
normalization applied to the repo's own dependencies — so `Flask` matches
`flask` (PyPI lowercases and collapses `-`/`_`/`.`), but on crates.io `-` and
`_` stay distinct. For Go, use the full module path
(`github.com/gin-gonic/gin`).

**File globs**: a pattern containing `/` is matched against the whole
repo-relative path with `*` not crossing a separator (`android/app/build.gradle`);
a bare pattern is matched against each file's basename (`*.tf`, `Dockerfile`).

**Predicates** (the only closed set — arbitrary names are ignored):

| Name | True when |
|---|---|
| `has_manifest_without_entrypoint` | the repo has a manifest but no recognizable program entrypoint (`src/main.rs`, `main.go`, `main.py`, `index.ts`, …) — the "this is a library, not an app" heuristic |

## Technology rules

```toml
[[tech]]
id = "acme-orm"
name = "Acme ORM"          # shown on the repo's Technologies card
kind = "framework"         # framework | tooling | package-manager | runtime
any_dependency = ["@acme/orm"]
any_file = ["acme.orm.config.ts"]
```

A technology is detected when `any_dependency` or `any_file` matches (same
matching rules as category signals). Each detection records **why**: a
dependency-confirmed hit renders solid; a marker-file-only hit renders dashed
and lower-prominence (FR-2.4). `id` merges the same way as category rules.

## Worked example

`~/.config/dev.repo-radar/rules/acme.toml`:

```toml
# Treat repos that use our internal web framework as Backend, strongly.
[[rule]]
id = "server-framework"                 # overrides the shipped rule of this id
weights = { Backend = 8 }
any_dependency = [
    "express", "fastify", "@nestjs/core",
    "axum", "django", "flask", "fastapi",
    "@acme/web",                        # …plus ours
]

# A house tool we want to see on the Technologies card.
[[tech]]
id = "acme-cli"
name = "Acme CLI"
kind = "tooling"
any_file = ["acme.toml", ".acme/config.yaml"]
```

Save it, run a scan, and open any affected repo's **Category** panel to see
the new weights take effect — no rebuild.
