//! Library surface of `markdoc-pdf`.

pub mod assets;
pub mod dates;
pub mod render;

/// Re-export of the typed Flux frontmatter view (its own crate so other
/// Adeptus consumers can reuse it without depending on the PDF stack).
pub use flux_types as flux;
