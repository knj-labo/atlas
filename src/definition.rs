use super::{ProjectFile, ProjectSnapshot};
use crate::{
    Binding, DeclarationId, Export, ImportSpecifier, ModuleResolution, Resolution, ScopeDomain,
};
use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
};

/// A definition borrowed from the queried snapshot, with its original path.
#[derive(Debug, Clone, Copy)]
pub struct Definition<'a> {
    pub path: &'a Path,
    pub kind: DefinitionKind<'a>,
}

#[derive(Debug, Clone, Copy)]
pub enum DefinitionKind<'a> {
    Declaration(&'a Binding),
    /// An anonymous default export or an exported expression.
    Export(&'a Export),
    AstroComponent(&'a ProjectFile),
    /// A namespace root, without member or type inference.
    Module(&'a ProjectFile),
}

impl Definition<'_> {
    /// Original UTF-8 byte range; component/module targets cover the whole file.
    pub fn span(&self) -> Range<u32> {
        match self.kind {
            DefinitionKind::Declaration(binding) => binding.span.clone(),
            DefinitionKind::Export(export) => export.span.clone(),
            DefinitionKind::AstroComponent(file) | DefinitionKind::Module(file) => {
                0..file.source.len().try_into().expect("Oxc source length")
            }
        }
    }

    fn same_target(&self, other: &Self) -> bool {
        self.path == other.path
            && match (self.kind, other.kind) {
                (DefinitionKind::Declaration(a), DefinitionKind::Declaration(b)) => a.id == b.id,
                (DefinitionKind::Export(a), DefinitionKind::Export(b)) => a.span == b.span,
                (DefinitionKind::AstroComponent(_), DefinitionKind::AstroComponent(_))
                | (DefinitionKind::Module(_), DefinitionKind::Module(_)) => true,
                _ => false,
            }
    }
}

/// Definition of a recorded reference. Ambiguous candidates are distinct,
/// confirmed targets in deterministic path/span order, never guessed bindings.
#[derive(Debug, Clone)]
pub enum DefinitionResolution<'a> {
    Resolved(Definition<'a>),
    Ambiguous(Vec<Definition<'a>>),
    Unresolved(DefinitionUnresolved),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum DefinitionUnresolved {
    UnresolvedReference,
    Builtin,
    RuntimeDependent,
    UnresolvedModule,
    UnsupportedModule,
    AmbiguousModule { candidates: Vec<PathBuf> },
    UnsupportedSourceType { path: PathBuf },
    AnalysisFailed { path: PathBuf },
    MissingExport { path: PathBuf, name: String },
    Cycle { path: PathBuf, name: String },
}

#[derive(Default)]
struct Search<'a> {
    targets: Vec<Definition<'a>>,
    failures: Vec<DefinitionUnresolved>,
}

impl<'a> Search<'a> {
    fn target(path: &'a Path, kind: DefinitionKind<'a>) -> Self {
        Self {
            targets: vec![Definition { path, kind }],
            failures: Vec::new(),
        }
    }

    fn failed(reason: DefinitionUnresolved) -> Self {
        Self {
            targets: Vec::new(),
            failures: vec![reason],
        }
    }

    fn merge(&mut self, other: Self) {
        for target in other.targets {
            if !self
                .targets
                .iter()
                .any(|existing| existing.same_target(&target))
            {
                self.targets.push(target);
            }
        }
        for reason in other.failures {
            if !self.failures.contains(&reason) {
                self.failures.push(reason);
            }
        }
    }

    fn finish(mut self) -> DefinitionResolution<'a> {
        self.targets.sort_by(|a, b| {
            (a.path, a.span().start, a.span().end).cmp(&(b.path, b.span().start, b.span().end))
        });
        if self.targets.len() > 1 {
            return DefinitionResolution::Ambiguous(self.targets);
        }
        // A star branch that is absent or revisits an active export cannot
        // introduce another declaration. An unopened/failed branch can.
        let blocking = self.failures.iter().find(|reason| {
            !matches!(
                reason,
                DefinitionUnresolved::MissingExport { .. } | DefinitionUnresolved::Cycle { .. }
            )
        });
        if let Some(reason) = blocking {
            return DefinitionResolution::Unresolved(reason.clone());
        }
        if let Some(target) = self.targets.pop() {
            return DefinitionResolution::Resolved(target);
        }
        let reason = self
            .failures
            .iter()
            .find(|r| matches!(r, DefinitionUnresolved::Cycle { .. }))
            .or(self.failures.first())
            .expect("an unsuccessful search has a reason");
        DefinitionResolution::Unresolved(reason.clone())
    }
}

impl ProjectSnapshot {
    /// Resolve a recorded reference to its declaration/export in this snapshot.
    ///
    /// Follows local bindings, named/default imports, import aliases, named
    /// re-exports and `export *` chains. Explicit exports take precedence; stars
    /// exclude `default`. Repeated paths to the same declaration are merged.
    /// Namespace roots return the target module, without resolving members.
    /// Unknown/failed files and offsets without a reference return `None`.
    /// This only traverses saved facts and module resolutions: no parsing,
    /// resolver execution, filesystem access, or type inference occurs.
    ///
    /// ```
    /// let source = "import { value as local } from './value.ts'; local;";
    /// let project = atlas::analyze_project([
    ///     ("value.ts", "export const value = 1;"), ("use.ts", source),
    /// ]);
    /// let result = project.definition_at("use.ts", source.rfind("local").unwrap() as u32);
    /// assert!(matches!(result, Some(atlas::DefinitionResolution::Resolved(d))
    ///     if d.path == std::path::Path::new("value.ts")));
    /// ```
    pub fn definition_at(
        &self,
        path: impl AsRef<Path>,
        offset: u32,
    ) -> Option<DefinitionResolution<'_>> {
        let reference = self.reference_at(path, offset)?;
        Some(match &reference.reference.resolution {
            Resolution::Resolved(binding) => self
                .definition_of_binding(reference.path, binding.id, &mut HashSet::new())
                .finish(),
            Resolution::Builtin(_) => {
                DefinitionResolution::Unresolved(DefinitionUnresolved::Builtin)
            }
            Resolution::Unresolved => {
                DefinitionResolution::Unresolved(DefinitionUnresolved::UnresolvedReference)
            }
            Resolution::RuntimeDependent { .. } => {
                DefinitionResolution::Unresolved(DefinitionUnresolved::RuntimeDependent)
            }
        })
    }

    fn definition_of_binding<'a>(
        &'a self,
        path: &'a Path,
        id: DeclarationId,
        visiting: &mut HashSet<(PathBuf, String)>,
    ) -> Search<'a> {
        let facts = self.files[path]
            .analysis
            .as_ref()
            .as_ref()
            .expect("analyzed binding");
        let binding = &facts.declarations[id.0];
        if let Some(target) = &binding.import {
            let dependency = self.indexes[path].bindings[&id];
            self.definition_in_module(
                &self.dependencies[dependency].resolution,
                &target.specifier,
                visiting,
            )
        } else {
            Search::target(path, DefinitionKind::Declaration(binding))
        }
    }

    fn definition_in_module<'a>(
        &'a self,
        resolution: &ModuleResolution,
        specifier: &ImportSpecifier,
        visiting: &mut HashSet<(PathBuf, String)>,
    ) -> Search<'a> {
        let target = match resolution {
            ModuleResolution::Resolved(path) => path,
            ModuleResolution::Unresolved => {
                return Search::failed(DefinitionUnresolved::UnresolvedModule);
            }
            ModuleResolution::Unsupported => {
                return Search::failed(DefinitionUnresolved::UnsupportedModule);
            }
            ModuleResolution::Ambiguous(candidates) => {
                return Search::failed(DefinitionUnresolved::AmbiguousModule {
                    candidates: candidates.clone(),
                });
            }
        };
        let (path, file) = self
            .files
            .get_key_value(target)
            .expect("registered resolution");
        if crate::analyze::is_commonjs(path) {
            return Search::failed(DefinitionUnresolved::UnsupportedSourceType {
                path: path.clone(),
            });
        }
        if file.analysis.is_err() {
            return Search::failed(DefinitionUnresolved::AnalysisFailed { path: path.clone() });
        }
        let name = match specifier {
            ImportSpecifier::Namespace => {
                return Search::target(path, DefinitionKind::Module(file));
            }
            ImportSpecifier::Default => "default",
            ImportSpecifier::Named(name) => name,
        };
        self.definition_of_export(path, name, visiting)
    }

    fn definition_of_export<'a>(
        &'a self,
        path: &'a Path,
        name: &str,
        visiting: &mut HashSet<(PathBuf, String)>,
    ) -> Search<'a> {
        let key = (path.to_path_buf(), name.to_owned());
        if !visiting.insert(key.clone()) {
            return Search::failed(DefinitionUnresolved::Cycle {
                path: path.to_path_buf(),
                name: name.into(),
            });
        }
        let file = &self.files[path];
        let facts = file.analysis.as_ref().as_ref().expect("analyzed export");
        let mut result = Search::default();
        if name == "default"
            && facts
                .scopes
                .iter()
                .any(|s| s.parent.is_none() && s.domain == ScopeDomain::Server)
        {
            result = Search::target(path, DefinitionKind::AstroComponent(file));
        } else {
            let explicit: Vec<_> = facts
                .exports
                .iter()
                .enumerate()
                .filter(|(_, e)| e.name == name && !e.is_star())
                .collect();
            if explicit.is_empty() {
                if name != "default" {
                    for edge in &self.indexes[path].reexports {
                        let edge = &self.reexports[*edge];
                        let export = &facts.exports[edge.index];
                        if export.is_star() {
                            result.merge(self.definition_in_module(
                                &edge.resolution,
                                &ImportSpecifier::Named(name.into()),
                                visiting,
                            ));
                        }
                    }
                }
            } else {
                for (index, export) in explicit {
                    if export.source.is_some() {
                        let edge = self.indexes[path]
                            .reexports
                            .iter()
                            .map(|&i| &self.reexports[i])
                            .find(|e| e.index == index)
                            .expect("indexed re-export");
                        result.merge(self.definition_in_module(
                            &edge.resolution,
                            export.imported.as_ref().expect("named re-export"),
                            visiting,
                        ));
                    } else if let Some(local) = export.local {
                        result.merge(self.definition_of_binding(path, local, visiting));
                    } else {
                        result.merge(Search::target(path, DefinitionKind::Export(export)));
                    }
                }
            }
        }
        visiting.remove(&key);
        if result.targets.is_empty() && result.failures.is_empty() {
            result = Search::failed(DefinitionUnresolved::MissingExport {
                path: path.to_path_buf(),
                name: name.into(),
            });
        }
        result
    }
}
