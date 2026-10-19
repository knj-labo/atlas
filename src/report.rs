use crate::project::DefinitionResolver;
use crate::{
    Definition, DefinitionKind, DefinitionResolution, DefinitionUnresolved, Diagnostic, ExportRole,
    ImportSpecifier, ModuleResolution, ProjectSnapshot, ReferenceKind, ScopeDomain,
};
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
    /// Lexical uses of import bindings, including unresolved module/export targets.
    pub references: Vec<ReferenceReport>,
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
    pub imported: Option<ImportSpecifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub key: String,
    pub source: String,
    pub domain: ScopeDomain,
    pub resolution: ModuleResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReferenceReport {
    pub key: String,
    pub name: String,
    pub kind: ReferenceKind,
    /// Key of the originating import declaration.
    pub import: String,
    pub definition: DefinitionReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DefinitionReport {
    Resolved(DefinitionTargetReport),
    Ambiguous(Vec<DefinitionTargetReport>),
    Unresolved(DefinitionUnresolved),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefinitionTargetReport {
    pub key: String,
    pub kind: DefinitionTargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DefinitionTargetKind {
    Declaration,
    Export,
    AstroComponent,
    Module,
}

impl From<Definition<'_>> for DefinitionTargetReport {
    fn from(definition: Definition<'_>) -> Self {
        Self {
            key: key(definition.path, &definition.span()),
            kind: match definition.kind {
                DefinitionKind::Declaration(_) => DefinitionTargetKind::Declaration,
                DefinitionKind::Export(_) => DefinitionTargetKind::Export,
                DefinitionKind::AstroComponent(_) => DefinitionTargetKind::AstroComponent,
                DefinitionKind::Module(_) => DefinitionTargetKind::Module,
            },
        }
    }
}

impl From<DefinitionResolution<'_>> for DefinitionReport {
    fn from(resolution: DefinitionResolution<'_>) -> Self {
        match resolution {
            DefinitionResolution::Resolved(target) => Self::Resolved(target.into()),
            DefinitionResolution::Ambiguous(targets) => {
                Self::Ambiguous(targets.into_iter().map(Into::into).collect())
            }
            DefinitionResolution::Unresolved(reason) => Self::Unresolved(reason),
        }
    }
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
        let mut resolver = DefinitionResolver::new(self);
        self.report_with_resolver(&mut resolver)
    }

    fn report_with_resolver<'a>(&'a self, resolver: &mut DefinitionResolver<'a>) -> ProjectReport {
        let mut component_uses = self.component_usage_map(resolver);
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
                                imported: export.imported.clone(),
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
                    references: file
                        .analysis
                        .as_ref()
                        .as_ref()
                        .ok()
                        .into_iter()
                        .flat_map(|facts| &facts.references)
                        .filter_map(|reference| {
                            let reference = self.project_reference(path, reference);
                            let import = reference.import?;
                            Some(ReferenceReport {
                                key: key(path, &reference.reference.span),
                                name: reference.reference.name.clone(),
                                kind: reference.reference.kind,
                                import: key(path, &import.import.span),
                                definition: resolver.reference(reference).into(),
                            })
                        })
                        .collect(),
                    component_uses: component_uses
                        .remove(path)
                        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_import_uses_share_one_finished_lookup_without_losing_occurrences() {
        let count = 16_000;
        let source = format!(
            "import {{ value }} from './value.ts';\n{}",
            "value;\n".repeat(count)
        );
        let project =
            crate::analyze_project([("value.ts", "export const value = 1;"), ("use.ts", &source)]);
        let mut resolver = DefinitionResolver::new(&project);
        let report = project.report_with_resolver(&mut resolver);
        let references = &report
            .files
            .iter()
            .find(|f| f.path == "use.ts")
            .unwrap()
            .references;
        assert_eq!(references.len(), count);
        assert_eq!(resolver.searches, 1);
        assert_eq!(
            references.first().unwrap().key,
            format!(
                "use.ts#{}-{}",
                source.find("\nvalue").unwrap() + 1,
                source.find("\nvalue").unwrap() + 6
            )
        );
        assert_eq!(
            references.last().unwrap().key,
            format!(
                "use.ts#{}-{}",
                source.rfind("value").unwrap(),
                source.rfind("value").unwrap() + 5
            )
        );
        assert!(references.iter().all(
            |r| matches!(&r.definition, DefinitionReport::Resolved(d) if d.key == "value.ts#13-18")
        ));
    }

    #[test]
    fn batch_cache_pairs_binding_ids_with_their_file() {
        let project = crate::analyze_project([
            ("first.ts", "export const value = 1;"),
            ("second.ts", "export const value = 2;"),
            ("a.ts", "import { value } from './first.ts'; value; value;"),
            ("b.ts", "import { value } from './second.ts'; value; value;"),
        ]);
        let mut resolver = DefinitionResolver::new(&project);
        let report = project.report_with_resolver(&mut resolver);
        assert_eq!(resolver.searches, 2);
        for (path, target) in [("a.ts", "first.ts#13-18"), ("b.ts", "second.ts#13-18")] {
            let file = report.files.iter().find(|f| f.path == path).unwrap();
            assert_eq!(file.references.len(), 2);
            assert!(file.references.iter().all(
                |r| matches!(&r.definition, DefinitionReport::Resolved(d) if d.key == target)
            ));
        }
    }
}
