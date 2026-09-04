//! Template rendering (M4-1, DESIGN §11.1).
//!
//! One `minijinja` environment, one entry point. Built-in and user templates
//! reach it the same way — there is no privileged built-in path.

use minijinja::{Environment, Value};

use crate::error::{CoreError, CoreResult};
use crate::model::Freshness;

use super::context::{freshness_phrase, PromptContext};

/// Render `template_src` against `ctx`. The template name is only used in
/// error messages.
pub fn render(name: &str, template_src: &str, ctx: &PromptContext) -> CoreResult<String> {
    // `trim_blocks`/`lstrip_blocks` are left OFF: the built-in templates
    // control their own whitespace with explicit `{%- -%}` markers, and a
    // user template (FR-9.2) behaves the same way a stock Jinja2 setup would.
    let mut env = Environment::new();

    // `{{ advisoryFreshness | freshness_phrase }}` → a readable sentence
    // instead of the bare enum name.
    env.add_filter("freshness_phrase", |v: Value| -> String {
        let phrase = match v.as_str() {
            Some("Never") => freshness_phrase(Freshness::Never),
            Some("Stale") => freshness_phrase(Freshness::Stale),
            Some("VeryStale") => freshness_phrase(Freshness::VeryStale),
            Some("Fresh") => freshness_phrase(Freshness::Fresh),
            _ => "unknown",
        };
        phrase.to_string()
    });

    env.add_template(name, template_src)
        .map_err(|e| CoreError::Prompt(format!("template {name} did not parse: {e}")))?;
    let tmpl = env
        .get_template(name)
        .map_err(|e| CoreError::Prompt(e.to_string()))?;

    let value = Value::from_serialize(ctx);
    tmpl.render(value).map_err(|e| {
        let chain = render_chain(&e);
        CoreError::Prompt(format!("rendering {name} failed: {chain}"))
    })
}

/// minijinja nests the useful part of a failure in the error's source chain;
/// flatten it so the message the user sees names the real problem.
fn render_chain(err: &minijinja::Error) -> String {
    let mut out = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(e) = source {
        out.push_str(&format!(": {e}"));
        source = e.source();
    }
    out
}

/// The token estimate the UI shows (FR-9.4): `chars / 4`, rounded up. This is
/// deliberately crude and is always labelled as an estimate in the UI — no
/// tokenizer ships in v1.
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    chars.div_ceil(4).min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Freshness;
    use crate::prompt::context::ScopeContext;
    use chrono::Utc;

    fn ctx() -> PromptContext {
        PromptContext {
            generated_at: Utc::now(),
            repos: vec![],
            scope: ScopeContext::WholeRepo,
            files: vec![],
            advisory_freshness: Freshness::Fresh,
        }
    }

    #[test]
    fn renders_a_trivial_template() {
        let out = render("t", "repos: {{ repos | length }}", &ctx()).unwrap();
        assert_eq!(out, "repos: 0");
    }

    #[test]
    fn parse_error_is_reported_with_the_template_name() {
        let err = render("bad", "{% for x in %}", &ctx()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bad"), "{msg}");
    }

    #[test]
    fn token_estimate_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
