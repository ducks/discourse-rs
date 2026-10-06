//! Discourse's markdown as PrettyText renders it (render), the sanitizer
//! that ends it, and the emoji data both read. Nothing here does I/O: the
//! server cooks posts with it, adding what it looks up in the database, and
//! the composer's preview runs the same code as WebAssembly (wasm), so the
//! preview is the post.

pub mod emoji;
pub mod render;
pub mod sanitizer;
#[cfg(target_arch = "wasm32")]
mod wasm;

/// A Discourse behavior this port doesn't cover yet, hit at runtime. Failing
/// loudly beats serving a plausible but wrong response in a parity port.
#[derive(Debug)]
pub struct Unsupported(pub &'static str);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not ported yet: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}
