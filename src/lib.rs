//! Owned facts from semantic analysis of a single Astro source file.
//!
//! ```
//! let facts = atlas::analyze("<Missing client:visible />").unwrap();
//! assert_eq!(facts.components.len(), 1);
//! assert_eq!(facts.components[0].reference.resolution, atlas::Resolution::Unresolved);
//! ```

mod analyze;
mod facts;
mod markup;
mod scripts;
mod semantic;
mod syntax;
mod validate;

pub use analyze::analyze;
pub use facts::*;
