//! Owned facts and read-only queries for Astro and ESM JavaScript/TypeScript.
//! Use [`analyze`] for Astro, [`analyze_file`] for path-selected syntax, or
//! [`analyze_project`] for import, declaration-definition and
//! file-usage queries over caller-supplied paths and source text. Neither needs
//! a cache. Enable `incremental` for an `AnalysisDatabase` that memoizes file
//! analyses and project snapshots.
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
mod boundary;
mod css;
mod css_source;
mod facts;
#[cfg(feature = "incremental")]
mod incremental;
mod markup;
mod project;
mod query;
#[cfg(feature = "serde")]
mod report;
mod resolver;
mod scripts;
mod semantic;
mod syntax;
mod transfer;
mod validate;

pub use analyze::{analyze, analyze_file};
pub use facts::*;
#[cfg(feature = "incremental")]
pub use incremental::AnalysisDatabase;
pub use project::{
    Definition, DefinitionKind, DefinitionResolution, DefinitionUnresolved, ProjectComponentUse,
    ProjectExport, ProjectFile, ProjectImport, ProjectReExport, ProjectReference, ProjectRevision,
    ProjectSnapshot, analyze_project, analyze_project_with_resolver,
};
#[cfg(feature = "serde")]
pub use report::{
    ComponentUseReport, DefinitionReport, DefinitionTargetKind, DefinitionTargetReport,
    ExportReport, FileReport, ImportReport, ProjectReport, REPORT_SCHEMA_VERSION, ReferenceReport,
};
pub use resolver::{
    ModuleDependency, ModuleRequest, ModuleResolution, ModuleResolver, ProjectSources,
    RelativeResolver,
};
pub use transfer::*;

#[cfg(test)]
#[path = "../tests/support/snapshots.rs"]
mod snapshots;
