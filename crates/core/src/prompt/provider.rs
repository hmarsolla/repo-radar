//! Phase 2 seam (M4-7, DESIGN §11.5, PRD H1/H3).
//!
//! v1 generates a prompt and hands it back as a `String`; what the user does
//! with it — clipboard, file, paste into a chat — is not this module's
//! concern. Phase 2 will add a provider that sends the prompt to an LLM and
//! streams a reply. Defining the trait now proves the generator is already
//! decoupled from delivery: nothing in [`super::build`] or
//! [`super::render`] refers to a destination.
//!
//! This trait is intentionally unused in v1. Per PRD H3 there is **no
//! disabled provider dropdown** in the UI — the seam is in the code, not on
//! the screen.

use std::future::Future;
use std::pin::Pin;

use crate::error::CoreResult;

/// A future resolving to the model's full response. Phase 2 will widen this
/// to a stream; the signature deliberately stays minimal so adding one is
/// additive.
pub type Completion<'a> = Pin<Box<dyn Future<Output = CoreResult<String>> + Send + 'a>>;

/// Something that can turn a rendered prompt into a model response. No
/// implementation ships in v1.
#[allow(dead_code)]
pub trait LlmProvider: Send + Sync {
    fn complete<'a>(&'a self, prompt: &'a str) -> Completion<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trait compiles and is object-safe. A trivial echo provider is
    /// enough to prove both without pulling in an async runtime.
    struct Echo;

    impl LlmProvider for Echo {
        fn complete<'a>(&'a self, prompt: &'a str) -> Completion<'a> {
            Box::pin(async move { Ok(prompt.to_string()) })
        }
    }

    #[test]
    fn provider_is_object_safe_and_decoupled_from_delivery() {
        let _boxed: Box<dyn LlmProvider> = Box::new(Echo);
    }
}
