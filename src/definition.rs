use super::{ProjectFile, ProjectReference, ProjectSnapshot};
use crate::{
    Binding, DeclarationId, Export, ImportSpecifier, ModuleResolution, Resolution, ScopeDomain,
};
use std::{
    collections::{HashMap, HashSet},
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
    active: HashSet<(&'a Path, String)>,
    visited: HashSet<(&'a Path, String)>,
    #[cfg(test)]
    export_visits: usize,
}

impl<'a> Search<'a> {
    fn target(&mut self, path: &'a Path, kind: DefinitionKind<'a>) {
        let target = Definition { path, kind };
        if !self
            .targets
            .iter()
            .any(|existing| existing.same_target(&target))
        {
            self.targets.push(target);
        }
    }

    fn failed(&mut self, reason: DefinitionUnresolved) {
        if !self.failures.contains(&reason) {
            self.failures.push(reason);
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

/// A batch shares only fully finished root results. Traversal state belongs to
/// a single root; partial SCC results must never be cached across roots.
pub(crate) struct DefinitionResolver<'a> {
    project: &'a ProjectSnapshot,
    bindings: HashMap<(&'a Path, DeclarationId), DefinitionResolution<'a>>,
    exports: HashMap<(&'a Path, String), DefinitionResolution<'a>>,
    #[cfg(test)]
    pub(crate) searches: usize,
}

impl<'a> DefinitionResolver<'a> {
    pub(crate) fn new(project: &'a ProjectSnapshot) -> Self {
        Self {
            project,
            bindings: HashMap::new(),
            exports: HashMap::new(),
            #[cfg(test)]
            searches: 0,
        }
    }

    pub(crate) fn reference(
        &mut self,
        reference: ProjectReference<'a>,
    ) -> DefinitionResolution<'a> {
        match &reference.reference.resolution {
            Resolution::Resolved(binding) => {
                let key = (reference.path, binding.id);
                if let Some(result) = self.bindings.get(&key) {
                    return result.clone();
                }
                #[cfg(test)]
                {
                    self.searches += 1;
                }
                let mut search = Search::default();
                self.project
                    .definition_of_binding(reference.path, binding.id, &mut search);
                let result = search.finish();
                self.bindings.insert(key, result.clone());
                result
            }
            Resolution::Builtin(_) => {
                DefinitionResolution::Unresolved(DefinitionUnresolved::Builtin)
            }
            Resolution::Unresolved => {
                DefinitionResolution::Unresolved(DefinitionUnresolved::UnresolvedReference)
            }
            Resolution::RuntimeDependent { .. } => {
                DefinitionResolution::Unresolved(DefinitionUnresolved::RuntimeDependent)
            }
        }
    }

    /// `path` is the canonical, successfully analyzed target of a namespace root.
    pub(crate) fn export(&mut self, path: &'a Path, name: &str) -> DefinitionResolution<'a> {
        let key = (path, name.to_owned());
        if let Some(result) = self.exports.get(&key) {
            return result.clone();
        }
        #[cfg(test)]
        {
            self.searches += 1;
        }
        let mut search = Search::default();
        self.project.definition_of_export(path, name, &mut search);
        let result = search.finish();
        self.exports.insert(key, result.clone());
        result
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
        Some(DefinitionResolver::new(self).reference(reference))
    }

    fn definition_of_binding<'a>(
        &'a self,
        path: &'a Path,
        id: DeclarationId,
        search: &mut Search<'a>,
    ) {
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
                search,
            );
        } else {
            search.target(path, DefinitionKind::Declaration(binding));
        }
    }

    fn definition_in_module<'a>(
        &'a self,
        resolution: &ModuleResolution,
        specifier: &ImportSpecifier,
        search: &mut Search<'a>,
    ) {
        let target = match resolution {
            ModuleResolution::Resolved(path) => path,
            ModuleResolution::Unresolved => {
                search.failed(DefinitionUnresolved::UnresolvedModule);
                return;
            }
            ModuleResolution::Unsupported => {
                search.failed(DefinitionUnresolved::UnsupportedModule);
                return;
            }
            ModuleResolution::Ambiguous(candidates) => {
                search.failed(DefinitionUnresolved::AmbiguousModule {
                    candidates: candidates.clone(),
                });
                return;
            }
        };
        let (path, file) = self
            .files
            .get_key_value(target)
            .expect("registered resolution");
        if let Err(error) = file.analysis.as_ref() {
            search.failed(
                if error
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_deref() == Some("atlas/unsupported-source-type"))
                {
                    DefinitionUnresolved::UnsupportedSourceType { path: path.clone() }
                } else {
                    DefinitionUnresolved::AnalysisFailed { path: path.clone() }
                },
            );
            return;
        }
        let name = match specifier {
            ImportSpecifier::Namespace => {
                search.target(path, DefinitionKind::Module(file));
                return;
            }
            ImportSpecifier::Default => "default",
            ImportSpecifier::Named(name) => name,
        };
        self.definition_of_export(path, name, search);
    }

    fn definition_of_export<'a>(&'a self, path: &'a Path, name: &str, search: &mut Search<'a>) {
        let key = (path, name.to_owned());
        if search.active.contains(&key) {
            search.failed(DefinitionUnresolved::Cycle {
                path: path.to_path_buf(),
                name: name.into(),
            });
            return;
        }
        if !search.visited.insert(key.clone()) {
            return;
        }
        #[cfg(test)]
        {
            search.export_visits += 1;
        }
        search.active.insert(key.clone());
        let file = &self.files[path];
        let facts = file.analysis.as_ref().as_ref().expect("analyzed export");
        if name == "default"
            && facts
                .scopes
                .iter()
                .any(|s| s.parent.is_none() && s.domain == ScopeDomain::Server)
        {
            search.target(path, DefinitionKind::AstroComponent(file));
        } else {
            let explicit: Vec<_> = facts
                .exports
                .iter()
                .enumerate()
                .filter(|(_, e)| e.name == name && !e.is_star())
                .collect();
            if explicit.is_empty() {
                let mut has_stars = false;
                if name != "default" {
                    for edge in &self.indexes[path].reexports {
                        let edge = &self.reexports[*edge];
                        if facts.exports[edge.index].is_star() {
                            has_stars = true;
                            self.definition_in_module(
                                &edge.resolution,
                                &ImportSpecifier::Named(name.into()),
                                search,
                            );
                        }
                    }
                }
                // A completed revisit contributes nothing: all of its terminals
                // already belong to this root's accumulator. Only a leaf lookup
                // with no applicable edge establishes an absent export.
                if !has_stars {
                    search.failed(DefinitionUnresolved::MissingExport {
                        path: path.to_path_buf(),
                        name: name.into(),
                    });
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
                        self.definition_in_module(
                            &edge.resolution,
                            export.imported.as_ref().expect("named re-export"),
                            search,
                        );
                    } else if let Some(local) = export.local {
                        self.definition_of_binding(path, local, search);
                    } else if export.default_expression {
                        search.target(path, DefinitionKind::Export(export));
                    } else {
                        search.failed(DefinitionUnresolved::UnresolvedReference);
                    }
                }
            }
        }
        search.active.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_star_graph_expands_each_export_once() {
        let depth = 24;
        let mut sources = vec![("value.ts".to_owned(), "export const value = 1;".to_owned())];
        for level in 0..depth {
            let source = if level == 0 {
                "export * from './value.ts';".to_owned()
            } else {
                format!(
                    "export * from './a{}.ts'; export * from './b{}.ts';",
                    level - 1,
                    level - 1
                )
            };
            sources.push((format!("a{level}.ts"), source.clone()));
            sources.push((format!("b{level}.ts"), source));
        }
        let project = crate::analyze_project(sources);
        let (path, _) = project.files.get_key_value(Path::new("a23.ts")).unwrap();
        let mut search = Search::default();
        project.definition_of_export(path, "value", &mut search);
        assert_eq!(search.export_visits, 2 * depth);
        assert!(search.active.is_empty());
        assert!(matches!(search.finish(), DefinitionResolution::Resolved(d)
            if d.path == Path::new("value.ts") && matches!(d.kind, DefinitionKind::Declaration(_))));
    }
}
