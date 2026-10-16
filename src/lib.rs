//! Owned facts from semantic analysis of a single Astro source file.
//! Enable the `incremental` feature for an `AnalysisDatabase` that memoizes
//! those facts for caller-supplied file paths and source text, and creates
//! immutable `ProjectSnapshot` results with direct import and file-usage queries.
//!
//! ```
//! let facts = atlas::analyze("<Missing client:visible />").unwrap();
//! assert_eq!(facts.components.len(), 1);
//! assert_eq!(facts.components[0].reference.resolution, atlas::Resolution::Unresolved);
//! ```
//!
//! Search declarations and their definite uses within the same result:
//!
//! ```
//! let source = "---\nimport Button from './Button.astro';\n---\n<Button />";
//! let facts = atlas::analyze(source)?;
//! let button = facts.declarations_named("Button")[0];
//! let uses = facts.references_to(button.id);
//! assert_eq!(uses.len(), 1);
//! let offset = source.find("<Button").unwrap() as u32 + 1;
//! let reference = facts.reference_at(offset).unwrap();
//! assert_eq!(reference, uses[0]);
//! assert!(matches!(&reference.resolution, atlas::Resolution::Resolved(binding) if binding.id == button.id));
//! # Ok::<(), atlas::AnalysisError>(())
//! ```
//!
//! Queries borrow existing records. Offsets are original-source UTF-8 bytes;
//! IDs must come from the same analysis result, including after source updates.

mod analyze;
mod css;
mod css_source;
mod facts;
#[cfg(feature = "incremental")]
mod incremental;
mod markup;
#[cfg(feature = "incremental")]
mod project;
mod query;
#[cfg(feature = "incremental")]
mod resolver;
mod scripts;
mod semantic;
mod syntax;
mod validate;

pub use analyze::analyze;
pub use facts::*;
#[cfg(feature = "incremental")]
pub use incremental::AnalysisDatabase;
#[cfg(feature = "incremental")]
pub use project::{ProjectFile, ProjectImport, ProjectReference, ProjectRevision, ProjectSnapshot};
#[cfg(feature = "incremental")]
pub use resolver::{
    ModuleRequest, ModuleResolution, ModuleResolver, ProjectSources, RelativeResolver,
};

#[cfg(test)]
#[path = "../tests/support/snapshots.rs"]
mod snapshots;
