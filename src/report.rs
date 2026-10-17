use crate::{Diagnostic, ExportRole, ModuleResolution, ProjectSnapshot, ScopeDomain};
use serde::Serialize;
use std::{ops::Range, path::Path};

/// Bumped on any incompatible change to the serialized shape.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Owned, serializable project summary for external analyzers and agents.
///
/// Every record carries a `key` of the form `path#start-end`, using original
/// UTF-8 byte offsets. Keys are stable for unchanged sources, unlike local IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectReport {
    pub schema_version: u32,
    pub files: Vec<FileReport>,
    /// Keys of exports reported by [`ProjectSnapshot::unused_exports`].
    pub unused_exports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileReport {
    pub path: String,
    /// `false` when analysis failed; diagnostics then hold the errors.
    pub analyzed: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub exports: Vec<ExportReport>,
    pub imports: Vec<ImportReport>,
    /// Uses of this file's component in other files.
    pub component_uses: Vec<ComponentUseReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportReport {
    pub key: String,
    pub name: String,
    pub role: ExportRole,
    pub type_only: bool,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub key: String,
    pub source: String,
    pub domain: ScopeDomain,
    pub resolution: ModuleResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentUseReport {
    pub key: String,
    pub name: String,
    /// Passed prop names, excluding directives and `slot`; spreads appear as `...`.
    pub props: Vec<String>,
    pub directives: Vec<String>,
    pub slot: Option<String>,
}

fn key(path: &Path, span: &Range<u32>) -> String {
    format!("{}#{}-{}", path.display(), span.start, span.end)
}

impl ProjectSnapshot {
    /// Summarize this snapshot as a [`ProjectReport`].
    pub fn report(&self) -> ProjectReport {
        let files = self
            .files()
            .map(|(path, file)| {
                let (analyzed, diagnostics, exports) = match file.analysis.as_ref() {
                    Ok(facts) => (
                        true,
                        facts.diagnostics.clone(),
                        facts
                            .exports
                            .iter()
                            .map(|export| ExportReport {
                                key: key(path, &export.span),
                                name: export.name.clone(),
                                role: export.role,
                                type_only: export.type_only,
                                source: export.source.clone(),
                            })
                            .collect(),
                    ),
                    Err(error) => (false, error.diagnostics.clone(), Vec::new()),
                };
                FileReport {
                    path: path.display().to_string(),
                    analyzed,
                    diagnostics,
                    exports,
                    imports: self
                        .outgoing_imports(path)
                        .into_iter()
                        .map(|import| ImportReport {
                            key: key(path, &import.import.span),
                            source: import.import.source.clone(),
                            domain: import.domain,
                            resolution: import.resolution.clone(),
                        })
                        .collect(),
                    component_uses: self
                        .component_uses_of(path)
                        .into_iter()
                        .map(|usage| ComponentUseReport {
                            key: key(usage.path, &usage.component.span),
                            name: usage.component.name.clone(),
                            props: usage
                                .element
                                .attributes
                                .iter()
                                .map(|a| a.name.as_deref().unwrap_or("..."))
                                .filter(|name| {
                                    *name != "slot"
                                        && !usage
                                            .component
                                            .directives
                                            .iter()
                                            .any(|d| d.name == *name)
                                })
                                .map(String::from)
                                .collect(),
                            directives: usage
                                .component
                                .directives
                                .iter()
                                .map(|d| d.name.clone())
                                .collect(),
                            slot: usage.element.slot.as_ref().and_then(|s| s.name.clone()),
                        })
                        .collect(),
                }
            })
            .collect();
        ProjectReport {
            schema_version: REPORT_SCHEMA_VERSION,
            files,
            unused_exports: self
                .unused_exports()
                .into_iter()
                .map(|e| key(e.path, &e.export.span))
                .collect(),
        }
    }
}
