//! Prompt generation commands (M4-6, DESIGN §12.1).
//!
//! The heavy lifting is all in `repo_radar_core::prompt`; this layer resolves
//! repo ids to on-disk paths, pulls the prune list and token budget from the
//! settings store, and enforces the one rule the core cannot (it has no
//! notion of "a scanned repo"): an export never writes inside one (FR-9.5).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;

use repo_radar_core::db::advisories::sync_status;
use repo_radar_core::db::repos as repo_db;
use repo_radar_core::prompt::{
    bounded_tree, build_context, embed_files, estimate_tokens, list_selectable, list_templates,
    load_template_source, render, EmbeddedFile, ExclusionReason, ScopeContext, SelectionListing,
    SelectionOptions, TemplateInfo, TreeEntry,
};

use crate::error::{CommandError, CommandResult};
use crate::settings::{Settings, SETTINGS_KEY, STORE_FILE};
use crate::state::AppState;

/// How deep the directory tree carried into a prompt goes, and its entry cap.
const TREE_MAX_DEPTH: u32 = 3;
const TREE_CAP: usize = 400;

/// Every template available (built-ins + `<config>/prompts/*.j2`).
#[tauri::command]
#[specta::specta]
pub fn list_prompt_templates(state: State<'_, AppState>) -> CommandResult<Vec<TemplateInfo>> {
    Ok(list_templates(&state.core.paths)?)
}

/// The file selection tree for one repo (FR-9.3). Every path the user cannot
/// pick is still here, with the reason.
#[tauri::command]
#[specta::specta]
pub fn prompt_file_listing(
    app: AppHandle,
    state: State<'_, AppState>,
    repo_id: i64,
) -> CommandResult<SelectionListing> {
    let root = repo_path(&state, repo_id)?;
    let opts = selection_opts(&app);
    Ok(list_selectable(Path::new(&root), &opts)?)
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePromptRequest {
    pub template_id: String,
    /// One id for a single-repo template, several for a cross-repo one. The
    /// first is the repo files are embedded from.
    pub repo_ids: Vec<i64>,
    pub scope: ScopeContext,
    /// Repo-relative paths (within the first repo) to embed. The frontend's
    /// checkbox tree produces this; the backend re-checks every exclusion.
    #[serde(default)]
    pub selected_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFileInfo {
    pub path: String,
    pub reason: ExclusionReason,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedPrompt {
    /// The full rendered prompt — shown to the user before any copy or
    /// export (FR-9.6).
    pub prompt: String,
    /// `chars / 4`, always labelled an estimate in the UI (FR-9.4).
    pub estimated_tokens: u32,
    /// The configured budget, for the over-budget warning.
    pub token_budget: u32,
    pub included_files: u32,
    /// Files that were asked for but withheld, each with its reason.
    pub skipped_files: Vec<SkippedFileInfo>,
    pub template_name: String,
}

/// Build the context, render the template, and return the full prompt plus a
/// token estimate. Does not copy or export anything.
#[tauri::command]
#[specta::specta]
pub fn generate_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    req: GeneratePromptRequest,
) -> CommandResult<GeneratedPrompt> {
    if req.repo_ids.is_empty() {
        return Err(CommandError::Internal {
            message: "select at least one repository".into(),
        });
    }

    let opts = selection_opts(&app);

    let conn = state.core.db.read()?;
    let freshness = sync_status(&conn)?.freshness;

    // Per-repo directory tree.
    let multi = req.repo_ids.len() > 1;
    let mut trees: HashMap<i64, Vec<TreeEntry>> = HashMap::new();
    let mut roots: HashMap<i64, PathBuf> = HashMap::new();
    for &id in &req.repo_ids {
        let root = repo_path(&state, id)?;
        let listing = list_selectable(Path::new(&root), &opts)?;
        trees.insert(id, bounded_tree(&listing, TREE_MAX_DEPTH, TREE_CAP));
        roots.insert(id, PathBuf::from(root));
    }

    // Files are only embedded from the first repo (single-repo templates).
    let (files, skipped): (Vec<EmbeddedFile>, Vec<SkippedFileInfo>) =
        if req.selected_paths.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let first = req.repo_ids[0];
            let root = &roots[&first];
            let prefix = if multi {
                repo_name(&state, first).ok()
            } else {
                None
            };
            let (embedded, skip) =
                embed_files(root, &req.selected_paths, &opts, prefix.as_deref())?;
            (
                embedded,
                skip.into_iter()
                    .map(|(path, reason)| SkippedFileInfo { path, reason })
                    .collect(),
            )
        };
    let included_files = files.len() as u32;

    let ctx = build_context(&conn, &req.repo_ids, req.scope, files, &trees, freshness)?;
    let src = load_template_source(&state.core.paths, &req.template_id)?;
    let prompt = render(&req.template_id, &src, &ctx)?;
    let estimated_tokens = estimate_tokens(&prompt);

    let template_name = list_templates(&state.core.paths)?
        .into_iter()
        .find(|t| t.id == req.template_id)
        .map(|t| t.name)
        .unwrap_or_else(|| req.template_id.clone());

    Ok(GeneratedPrompt {
        prompt,
        estimated_tokens,
        token_budget: read_settings(&app).token_budget,
        included_files,
        skipped_files: skipped,
        template_name,
    })
}

/// Write a rendered prompt to a user-chosen path. Refuses any path inside a
/// scan root or a scanned repository (FR-9.5, Principle 4) — the frontend
/// picks the path with the save dialog; this is the backstop.
#[tauri::command]
#[specta::specta]
pub fn export_prompt(
    state: State<'_, AppState>,
    path: String,
    contents: String,
) -> CommandResult<()> {
    let target = PathBuf::from(&path);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    // Canonicalize the parent (it exists; the file may not) so the
    // containment check cannot be fooled by `..` or a symlink.
    let canon_parent = std::fs::canonicalize(parent).map_err(|e| CommandError::Internal {
        message: format!("cannot resolve {}: {e}", parent.display()),
    })?;

    for forbidden in protected_roots(&state)? {
        if let Ok(canon_forbidden) = std::fs::canonicalize(&forbidden) {
            if canon_parent == canon_forbidden || canon_parent.starts_with(&canon_forbidden) {
                return Err(CommandError::Internal {
                    message: format!(
                        "refusing to write inside a scanned location ({}). Choose a path outside your scan roots.",
                        forbidden.display()
                    ),
                });
            }
        }
    }

    std::fs::write(&target, contents.as_bytes()).map_err(|e| CommandError::Internal {
        message: format!("could not write {}: {e}", target.display()),
    })?;
    tracing::info!(path = %target.display(), "prompt exported");
    Ok(())
}

// -- helpers ---------------------------------------------------------------

fn repo_path(state: &State<'_, AppState>, repo_id: i64) -> CommandResult<String> {
    let conn = state.core.db.read()?;
    conn.query_row("SELECT path FROM repos WHERE id = ?1", [repo_id], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|_| CommandError::Internal {
        message: format!("repository {repo_id} not found"),
    })
}

fn repo_name(state: &State<'_, AppState>, repo_id: i64) -> CommandResult<String> {
    let conn = state.core.db.read()?;
    conn.query_row("SELECT name FROM repos WHERE id = ?1", [repo_id], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|_| CommandError::Internal {
        message: format!("repository {repo_id} not found"),
    })
}

/// Scan roots plus every scanned repo path — nothing an export may land in.
fn protected_roots(state: &State<'_, AppState>) -> CommandResult<Vec<PathBuf>> {
    let conn = state.core.db.read()?;
    let mut out: Vec<PathBuf> = repo_db::list_scan_roots(&conn)?
        .into_iter()
        .map(|r| PathBuf::from(r.path))
        .collect();
    out.extend(
        repo_db::all_fingerprints(&conn)?
            .into_iter()
            .map(|(path, _)| PathBuf::from(path)),
    );
    Ok(out)
}

fn read_settings(app: &AppHandle) -> Settings {
    app.store(STORE_FILE)
        .ok()
        .and_then(|s| s.get(SETTINGS_KEY))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn selection_opts(app: &AppHandle) -> SelectionOptions {
    let settings = read_settings(app);
    let mut opts = SelectionOptions {
        max_file_bytes: repo_radar_core::prompt::DEFAULT_MAX_FILE_BYTES,
        ..SelectionOptions::default()
    };
    // `prune_list` is *additional* (FR-10.1) — the built-ins always apply.
    for dir in settings.prune_list {
        let dir = dir.trim().to_string();
        if !dir.is_empty() && !opts.prune_dirs.contains(&dir) {
            opts.prune_dirs.push(dir);
        }
    }
    opts.excluded_extensions = settings
        .excluded_extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    opts
}
