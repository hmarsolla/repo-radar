# Prompt template context (FR-9.2)

repo-radar renders prompt templates with [minijinja][], a Jinja2-compatible
engine. Built-in templates live in `crates/core/assets/prompts/*.j2`; your own
templates go in `<config>/prompts/*.j2` and appear in the picker automatically
(the file stem becomes the template name). A user template whose stem matches a
built-in id shadows it.

The engine runs sandboxed: a template has no filesystem, network, or environment
access. It renders against **one** value, described below. Field names are
`snake_case` (the Jinja convention); the one exception is `scope`, whose `kind`
is `camelCase` because it doubles as an app command argument.

## Top level

| Field | Type | Notes |
|---|---|---|
| `generated_at` | RFC 3339 string | When the prompt was generated. |
| `repos` | list of *Repo* | One entry for a single-repo template, N for a cross-repo one. `repos[0]` is the convention for single-repo templates. |
| `scope` | *Scope* | What the user chose to include. |
| `files` | list of *File* | File bodies actually embedded, after exclusion (see below). Empty when the template does not use files. |
| `advisory_freshness` | `"Never" \| "Fresh" \| "Stale" \| "VeryStale"` | How current the advisory data behind the findings is. Pipe through the `freshness_phrase` filter for a sentence. |

## Repo

| Field | Type | Notes |
|---|---|---|
| `name` | string | |
| `category` | `"Frontend" \| "Backend" \| "Fullstack" \| "Mobile" \| "DevOps" \| "DataMl" \| "Library" \| "Cli" \| "Docs" \| "Unknown"` | Effective category — a manual override wins over the computed one. |
| `category_signals` | list of string | Plain lines describing the rules that drove the category, e.g. `"backend-express (dependency:express) → Backend +7.0"`. |
| `languages` | list of *Language* | Sorted by share descending. |
| `technologies` | list of *Technology* | Dependency-confirmed first. |
| `direct_dependencies` | list of *Dependency* | Direct deps only, each annotated with matched advisory ids. |
| `dependency_counts` | *DependencyCounts* | Direct/transitive totals, and a per-ecosystem breakdown. |
| `findings` | list of *Finding* | Security findings, compromise first. |
| `health` | *Health* or `null` | `null` when no advisory sync has ever completed (health is *unknown*, not healthy). |
| `git` | *Git* or `null` | `null` for a bare repo or a failed git read. |
| `tree` | list of *TreeEntry* | Bounded-depth directory listing. |

### Language
`language`, `code_lines`, `comment_lines`, `files`, `percentage` (0–100).

### Technology
`tech`, `kind` (`framework | tooling | package-manager | runtime`), `evidence`
(list of strings, each prefixed `dependency:` or `file:`).

### Dependency
`ecosystem`, `name`, `version`, `scope` (`runtime | dev | build | optional |
peer`), `version_confidence` (`exact` from a lockfile, `range` from a manifest —
a `range` dependency's findings are speculative), `advisories` (list of ids).

### DependencyCounts
`direct_total`, `transitive_total`, `per_ecosystem` (list of `{ ecosystem,
direct, transitive }`).

### Finding
`advisory_id`, `kind` (`compromise | vulnerability`), `severity` (`critical |
high | medium | low | unscored`), `package`, `version`, `fixed_version` (or
`null`), `summary`, `confirmed` (`true` when matched to an exact locked version;
`false` is the "speculative" bucket).

### Health
`score` (0–100), `band` (`critical | poor | fair | good | excellent`).

### Git
`head_sha`, `branch`, `last_commit_at`, `last_commit_summary`, `commits_90d`,
`commits_total`, `author_count`, `dirty_modified`, `dirty_staged`,
`dirty_untracked`, `ahead`, `behind`, `remote_url`, `branch_count`, `has_stash`.
Every field may be `null`.

### TreeEntry
`path` (repo-relative, `/`-separated), `is_dir`, `depth` (0 at top level).

## Scope

A tagged object. `scope.kind` is one of:

| `kind` | Extra fields | Meaning |
|---|---|---|
| `wholeRepo` | — | Everything selectable in the repo. |
| `directory` | `path` | A single subtree. |
| `files` | `paths` (list) | An explicit file list. |
| `diff` | `description` | A diff supplied as text. |

## File

`path` (repo-relative, `/`-separated; prefixed with the repo name when a prompt
spans several repos), `language` (a coarse guess from the extension, or `null`),
`content`, `bytes`.

Files are excluded automatically and never silently: binary files
(content-sniffed, not extension-guessed), files over 256 KB, gitignored files,
anything under a pruned directory, and any extension on the exclusion list in
**Settings → Prompts**. The picker shows each excluded path with its reason;
`files` only ever contains what survived.

## Filters

Standard minijinja builtins are available (`length`, `join`, `upper`, `lower`,
`round`, `default`, slicing `xs[:5]`, `loop.last`, …). repo-radar adds:

| Filter | Use |
|---|---|
| `freshness_phrase` | `{{ advisory_freshness | freshness_phrase }}` → a readable sentence. |

[minijinja]: https://docs.rs/minijinja
