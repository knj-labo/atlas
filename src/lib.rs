//! Owned facts from semantic analysis of a single Astro source file.
//! Enable the `incremental` feature for an `AnalysisDatabase` that memoizes
//! those facts for caller-supplied file paths and source text.
//!
//! ```
//! let facts = atlas::analyze("<Missing client:visible />").unwrap();
//! assert_eq!(facts.components.len(), 1);
//! assert_eq!(facts.components[0].reference.resolution, atlas::Resolution::Unresolved);
//! ```

mod analyze;
mod css;
mod css_source;
mod facts;
#[cfg(feature = "incremental")]
mod incremental;
mod markup;
mod scripts;
mod semantic;
mod syntax;
mod validate;

pub use analyze::analyze;
pub use facts::*;
#[cfg(feature = "incremental")]
pub use incremental::AnalysisDatabase;

#[cfg(test)]
#[path = "../tests/support/snapshots.rs"]
mod snapshots;
