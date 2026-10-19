use crate::{
    AnalysisError, ComponentUse, DeclarationId, Element, Export, ExportRole, FileFacts, Import,
    ImportSpecifier, ModuleDependency, ModuleRequest, ModuleResolution, ModuleResolver,
    ProjectSources, Reference, ReferenceId, RelativeResolver, Resolution, ScopeDomain,
    analyze_file,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[path = "definition.rs"]
mod definition;
pub(crate) use definition::DefinitionResolver;
pub use definition::{Definition, DefinitionKind, DefinitionResolution, DefinitionUnresolved};

/// Analyze Astro/ESM sources and index their imports without a cache.
///
/// Available with default features. Each call analyzes all supplied files and
/// resolves imports using [`RelativeResolver`]. The returned snapshot owns its
/// sources and results; later queries do not parse or resolve anything again.
/// Paths follow [`PathBuf`] equality, with the last source winning for duplicate
/// keys. No filesystem access is performed. Syntax errors are retained in each
/// [`ProjectFile`], and [`ProjectSnapshot::revision`] returns `None`.
///
/// ```
/// let project = atlas::analyze_project([
///     ("Button.astro", "<button />"),
///     ("Page.astro", "---\nimport Button from './Button.astro';\n---\n<Button />"),
/// ]);
/// assert_eq!(project.references_to_file("Button.astro").len(), 1);
/// assert_eq!(project.revision(), None);
/// ```
pub fn analyze_project<P, S>(sources: impl IntoIterator<Item = (P, S)>) -> ProjectSnapshot
where
    P: Into<PathBuf>,
    S: Into<Arc<str>>,
{
    analyze_project_with_resolver(sources, &RelativeResolver)
}

/// Analyze Astro/ESM sources without a cache, using a caller-supplied resolver.
///
/// The input, ownership, and error contracts match [`analyze_project`]. The
/// resolver is borrowed only during this call and is not retained in the
/// snapshot. Each call analyzes the final source for each path and resolves all
/// of its static imports anew, even when the input matches an earlier call.
pub fn analyze_project_with_resolver<P, S>(
    sources: impl IntoIterator<Item = (P, S)>,
    resolver: &dyn ModuleResolver,
) -> ProjectSnapshot
where
    P: Into<PathBuf>,
    S: Into<Arc<str>>,
{
    let mut unique_sources: BTreeMap<PathBuf, Arc<str>> = BTreeMap::new();
    for (path, source) in sources {
        unique_sources.insert(path.into(), source.into());
    }
    let files = unique_sources
        .into_iter()
        .map(|(path, source)| {
            let analysis = Arc::new(analyze_file(&path, &source));
            (path, ProjectFile { source, analysis })
        })
        .collect();
    ProjectSnapshot::build(None, files, resolver)
}

/// Database-local versions of the source set and module resolver settings.
/// Revisions from different databases are not comparable identities.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProjectRevision {
    pub sources: u64,
    pub resolver: u64,
}

/// A source buffer and the analysis of exactly that buffer.
#[derive(Debug, Clone)]
pub struct ProjectFile {
    pub source: Arc<str>,
    pub analysis: Arc<Result<FileFacts, AnalysisError>>,
}

/// A borrowed import, its containing file, and its module resolution.
#[derive(Debug, Clone, Copy)]
pub struct ProjectImport<'a> {
    /// Path of the file containing this import declaration.
    pub path: &'a Path,
    pub import: &'a Import,
    pub domain: ScopeDomain,
    pub resolution: &'a ModuleResolution,
}

/// A borrowed identifier reference with an optional originating import.
///
/// `reference.resolution` remains the file-local lexical result. `import` is
/// present only for a definite reference to an import binding, including when
/// that import's module is unresolved or unsupported. A resolved module does
/// not establish the existence or identity of any named export.
#[derive(Debug, Clone, Copy)]
pub struct ProjectReference<'a> {
    pub path: &'a Path,
    pub reference: &'a Reference,
    pub import: Option<ProjectImport<'a>>,
}

/// A template use of a component imported from another file.
#[derive(Debug, Clone, Copy)]
pub struct ProjectComponentUse<'a> {
    /// Path of the file containing the component use.
    pub path: &'a Path,
    pub component: &'a ComponentUse,
    /// The element carrying the passed attributes and slot assignment.
    pub element: &'a Element,
    pub import: ProjectImport<'a>,
}

/// A borrowed frontmatter export and its containing file.
#[derive(Debug, Clone, Copy)]
pub struct ProjectExport<'a> {
    pub path: &'a Path,
    pub export: &'a Export,
}

/// A re-export, its containing file, and the resolution of its module.
#[derive(Debug, Clone, Copy)]
pub struct ProjectReExport<'a> {
    /// Path of the file containing the re-export.
    pub path: &'a Path,
    pub export: &'a Export,
    pub resolution: &'a ModuleResolution,
}

#[derive(Debug)]
struct ReExportEdge {
    path: PathBuf,
    index: usize,
    resolution: ModuleResolution,
}

#[derive(Debug)]
struct Dependency {
    path: PathBuf,
    index: usize,
    domain: ScopeDomain,
    resolution: ModuleResolution,
}

#[derive(Debug, Default)]
struct FileIndex {
    imports: Vec<usize>,
    reexports: Vec<usize>,
    bindings: HashMap<DeclarationId, usize>,
    file_id: usize,
    exports: HashMap<String, Vec<usize>>,
    stars: Vec<usize>,
    export_edges: HashMap<usize, usize>,
    used_names: HashSet<String>,
    used_namespace: bool,
    used_star: bool,
}

#[derive(Debug)]
struct ImportUse {
    dependency: usize,
    reference: ReferenceId,
}

/// Immutable file analyses and direct import indexes for one source snapshot.
///
/// Created by [`analyze_project`] or [`analyze_project_with_resolver`], or by
/// `AnalysisDatabase::analyze_project` with the optional `incremental` feature.
/// Sources, local facts, and resolutions all belong to this snapshot. It remains
/// usable after database updates or destruction. Returned records borrow this
/// snapshot; local IDs must be paired with their file and this result.
///
/// Imports and references are ordered by path, then original-source position.
/// Failed files retain their `AnalysisError` and contribute no outgoing imports
/// or references. Incoming imports can still resolve to those registered files.
/// This indexes static import declarations, re-exports, and lexical uses of
/// imports, not dynamic imports, script `src`, or CSS dependencies.
#[derive(Debug)]
pub struct ProjectSnapshot {
    revision: Option<ProjectRevision>,
    files: BTreeMap<PathBuf, ProjectFile>,
    dependencies: Vec<Dependency>,
    indexes: BTreeMap<PathBuf, FileIndex>,
    importers: BTreeMap<PathBuf, Vec<usize>>,
    references: BTreeMap<PathBuf, Vec<ImportUse>>,
    reexports: Vec<ReExportEdge>,
    reexporters: BTreeMap<PathBuf, Vec<usize>>,
    file_paths: Vec<PathBuf>,
    definitions: Mutex<definition::ResolutionCache>,
    component_uses: OnceLock<BTreeMap<usize, Vec<(usize, usize)>>>,
}

impl ProjectSnapshot {
    /// Database-local source and resolver versions, or `None` for an uncached
    /// analysis. Versions from separate databases are not comparable.
    pub fn revision(&self) -> Option<ProjectRevision> {
        self.revision
    }

    pub fn files(&self) -> impl ExactSizeIterator<Item = (&Path, &ProjectFile)> {
        self.files.iter().map(|(path, file)| (path.as_path(), file))
    }

    pub fn file(&self, path: impl AsRef<Path>) -> Option<&ProjectFile> {
        self.files.get(path.as_ref())
    }

    /// Static import declarations contained in `path`, including unused and
    /// side-effect imports, regardless of their module resolution state.
    /// Unknown or failed files return no matches; inspect [`Self::file`] for errors.
    pub fn outgoing_imports(&self, path: impl AsRef<Path>) -> Vec<ProjectImport<'_>> {
        self.indexes
            .get(path.as_ref())
            .into_iter()
            .flat_map(|index| &index.imports)
            .map(|&index| self.import(index))
            .collect()
    }

    /// Static import declarations whose module resolved to the exact supplied
    /// target `path`. Each result identifies the file containing the declaration.
    pub fn incoming_imports(&self, path: impl AsRef<Path>) -> Vec<ProjectImport<'_>> {
        self.importers
            .get(path.as_ref())
            .into_iter()
            .flatten()
            .map(|&index| self.import(index))
            .collect()
    }

    /// Definite lexical uses of import bindings whose module resolved to `path`.
    ///
    /// Includes value/type uses, local aliases, and namespace-root references.
    /// This is a file-level usage query, not a query for an exported declaration.
    /// Unknown paths, unresolved/unsupported/ambiguous modules, and uncertain
    /// lexical resolutions produce no matches. Side-effect and unused imports
    /// are available through [`Self::incoming_imports`] instead.
    pub fn references_to_file(&self, path: impl AsRef<Path>) -> Vec<ProjectReference<'_>> {
        self.references
            .get(path.as_ref())
            .into_iter()
            .flatten()
            .map(|usage| {
                let import = self.import(usage.dependency);
                let facts = self.files[import.path]
                    .analysis
                    .as_ref()
                    .as_ref()
                    .expect("indexed facts");
                ProjectReference {
                    path: import.path,
                    reference: &facts.references[usage.reference.0],
                    import: Some(import),
                }
            })
            .collect()
    }

    /// Re-exports contained in `path`, regardless of their resolution state.
    pub fn outgoing_reexports(&self, path: impl AsRef<Path>) -> Vec<ProjectReExport<'_>> {
        self.indexes
            .get(path.as_ref())
            .into_iter()
            .flat_map(|index| &index.reexports)
            .map(|&edge| self.reexport(edge))
            .collect()
    }

    /// Re-exports whose module resolved to the exact supplied target `path`.
    pub fn incoming_reexports(&self, path: impl AsRef<Path>) -> Vec<ProjectReExport<'_>> {
        self.reexporters
            .get(path.as_ref())
            .into_iter()
            .flatten()
            .map(|&edge| self.reexport(edge))
            .collect()
    }

    /// Template component uses attributed to their resolved definition file.
    ///
    /// Uses the same saved export graph as [`Self::definition_at`], including
    /// imported-name/default re-exports, explicit priority and ambiguity. Static
    /// namespace members follow module exports; ordinary object members are not
    /// inferred. Direct namespace imports of Astro retain their file-level use
    /// contract for a single member. Unresolved and ambiguous targets are omitted.
    pub fn component_uses_of(&self, path: impl AsRef<Path>) -> Vec<ProjectComponentUse<'_>> {
        let Some(index) = self.indexes.get(path.as_ref()) else {
            return Vec::new();
        };
        self.component_usage_index(&mut DefinitionResolver::new(self))
            .get(&index.file_id)
            .into_iter()
            .flatten()
            .map(|&(file, component)| self.component_use(file, component))
            .collect()
    }

    fn component_usage_index<'a>(
        &'a self,
        resolver: &mut DefinitionResolver<'a>,
    ) -> &'a BTreeMap<usize, Vec<(usize, usize)>> {
        self.component_uses
            .get_or_init(|| self.compute_component_usage_map(resolver))
    }

    fn component_use(&self, file: usize, component: usize) -> ProjectComponentUse<'_> {
        let path = self.file_paths[file].as_path();
        let facts = self.files[path]
            .analysis
            .as_ref()
            .as_ref()
            .expect("component facts");
        let component = &facts.components[component];
        ProjectComponentUse {
            path,
            component,
            element: &facts.elements[component.element.0],
            import: self
                .project_reference(path, &facts.references[component.reference.id.0])
                .import
                .expect("imported component"),
        }
    }

    #[cfg(feature = "serde")]
    pub(crate) fn component_usage_map<'a>(
        &'a self,
        resolver: &mut DefinitionResolver<'a>,
    ) -> BTreeMap<&'a Path, Vec<ProjectComponentUse<'a>>> {
        self.component_usage_index(resolver)
            .iter()
            .map(|(target, uses)| {
                (
                    self.file_paths[*target].as_path(),
                    uses.iter()
                        .map(|&(file, component)| self.component_use(file, component))
                        .collect(),
                )
            })
            .collect()
    }

    fn compute_component_usage_map<'a>(
        &'a self,
        resolver: &mut DefinitionResolver<'a>,
    ) -> BTreeMap<usize, Vec<(usize, usize)>> {
        let mut uses: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
        for (path, file) in &self.files {
            let Ok(facts) = file.analysis.as_ref() else {
                continue;
            };
            for (component_index, component) in facts.components.iter().enumerate() {
                let reference =
                    self.project_reference(path, &facts.references[component.reference.id.0]);
                let Some(import) = reference.import else {
                    continue;
                };
                let DefinitionResolution::Resolved(mut target) = resolver.reference(reference)
                else {
                    continue;
                };
                if matches!(target.kind, DefinitionKind::Module(_)) {
                    let Some(members) = component
                        .name
                        .strip_prefix(&reference.reference.name)
                        .and_then(|name| name.strip_prefix('.'))
                    else {
                        continue;
                    };
                    let direct_namespace_member = !members.contains('.')
                        && matches!(import.resolution, ModuleResolution::Resolved(path) if path == target.path)
                        && matches!(&reference.reference.resolution, Resolution::Resolved(binding)
                            if binding.import.as_ref().is_some_and(|target| target.specifier == ImportSpecifier::Namespace));
                    let mut resolved = true;
                    let mut members = members.split('.').peekable();
                    while let Some(member) = members.next() {
                        let DefinitionKind::Module(module) = target.kind else {
                            resolved = false;
                            break;
                        };
                        match resolver.export(target.path, member) {
                            DefinitionResolution::Resolved(next) => target = next,
                            DefinitionResolution::Unresolved(
                                DefinitionUnresolved::MissingExport { .. },
                            ) if direct_namespace_member
                                && members.peek().is_none()
                                && module.analysis.as_ref().as_ref().is_ok_and(|facts| {
                                    facts.scopes.iter().any(|s| {
                                        s.parent.is_none() && s.domain == ScopeDomain::Server
                                    })
                                }) =>
                            {
                                // Legacy direct Astro namespace members are file-level
                                // candidates, without claiming a member declaration.
                                let DefinitionResolution::Resolved(next) =
                                    resolver.export(target.path, "default")
                                else {
                                    unreachable!("Astro default")
                                };
                                target = next;
                            }
                            _ => {
                                resolved = false;
                                break;
                            }
                        }
                    }
                    if !resolved || matches!(target.kind, DefinitionKind::Module(_)) {
                        continue;
                    }
                } else if component.name != reference.reference.name {
                    continue;
                }
                uses.entry(self.indexes[target.path].file_id)
                    .or_default()
                    .push((self.indexes[path].file_id, component_index));
            }
        }
        for uses in uses.values_mut() {
            uses.sort_unstable();
        }
        uses
    }

    /// Exports with role [`ExportRole::Other`] that no resolved import or
    /// re-export names.
    ///
    /// Namespace imports and `export * as` use every export. `export *` uses
    /// named exports but excludes `default`. Default imports use only `default`.
    /// A re-export counts as a use of its target; the
    /// re-exporting entry is itself reported when nothing uses it. Astro and
    /// Props exports are read by Astro itself, and `export *` entries have no
    /// name, so neither is reported. Unresolved modules and unregistered
    /// consumers are not seen, so callers should treat results as candidates.
    pub fn unused_exports(&self) -> Vec<ProjectExport<'_>> {
        let mut unused = Vec::new();
        for (path, file) in &self.files {
            let Ok(facts) = file.analysis.as_ref() else {
                continue;
            };
            let index = &self.indexes[path];
            unused.extend(
                facts
                    .exports
                    .iter()
                    .filter(|export| {
                        export.role == ExportRole::Other
                            && !export.is_star()
                            && !index.used_namespace
                            && (!index.used_star || export.name == "default")
                            && !index.used_names.contains(&export.name)
                    })
                    .map(|export| ProjectExport { path, export }),
            );
        }
        unused
    }

    /// Find a file-local reference and its originating import, if any.
    /// Offsets are original-source UTF-8 bytes, with the same half-open contract
    /// as [`FileFacts::reference_at`]. Unknown/failed files return `None`.
    pub fn reference_at(
        &self,
        path: impl AsRef<Path>,
        offset: u32,
    ) -> Option<ProjectReference<'_>> {
        let (path, file) = self.files.get_key_value(path.as_ref())?;
        let reference = file.analysis.as_ref().as_ref().ok()?.reference_at(offset)?;
        Some(self.project_reference(path, reference))
    }

    /// Wrap an already recorded canonical reference without scanning by offset.
    pub(crate) fn project_reference<'a>(
        &'a self,
        path: &'a Path,
        reference: &'a Reference,
    ) -> ProjectReference<'a> {
        let import = match &reference.resolution {
            Resolution::Resolved(binding) => self.indexes[path]
                .bindings
                .get(&binding.id)
                .map(|&index| self.import(index)),
            _ => None,
        };
        ProjectReference {
            path,
            reference,
            import,
        }
    }

    fn reexport(&self, edge: usize) -> ProjectReExport<'_> {
        let edge = &self.reexports[edge];
        let facts = self.files[&edge.path]
            .analysis
            .as_ref()
            .as_ref()
            .expect("indexed facts");
        ProjectReExport {
            path: &edge.path,
            export: &facts.exports[edge.index],
            resolution: &edge.resolution,
        }
    }

    fn import(&self, index: usize) -> ProjectImport<'_> {
        let dependency = &self.dependencies[index];
        let facts = self.files[&dependency.path]
            .analysis
            .as_ref()
            .as_ref()
            .expect("indexed facts");
        ProjectImport {
            path: &dependency.path,
            import: &facts.imports[dependency.index],
            domain: dependency.domain,
            resolution: &dependency.resolution,
        }
    }

    pub(crate) fn build(
        revision: Option<ProjectRevision>,
        files: BTreeMap<PathBuf, ProjectFile>,
        resolver: &dyn ModuleResolver,
    ) -> Self {
        let sources = files
            .iter()
            .map(|(path, file)| (path.clone(), Arc::clone(&file.source)))
            .collect();
        let context = ProjectSources::new(&sources);
        let mut snapshot = Self {
            revision,
            files,
            dependencies: Vec::new(),
            indexes: BTreeMap::new(),
            importers: BTreeMap::new(),
            references: BTreeMap::new(),
            reexports: Vec::new(),
            reexporters: BTreeMap::new(),
            file_paths: sources.keys().cloned().collect(),
            definitions: Mutex::new(definition::ResolutionCache::default()),
            component_uses: OnceLock::new(),
        };
        let resolve = |request: ModuleRequest<'_>| {
            let resolution = resolver.resolve(request, &context);
            match resolution {
                ModuleResolution::Resolved(target) if !context.contains(&target) => {
                    ModuleResolution::Unresolved
                }
                resolution => resolution,
            }
        };
        for (file_id, (path, file)) in snapshot.files.iter().enumerate() {
            let Ok(facts) = file.analysis.as_ref() else {
                continue;
            };
            let mut index = FileIndex {
                file_id,
                ..FileIndex::default()
            };
            for (i, export) in facts.exports.iter().enumerate() {
                if export.is_star() {
                    index.stars.push(i);
                } else {
                    index
                        .exports
                        .entry(export.name.clone())
                        .or_default()
                        .push(i);
                }
            }
            for (import_index, import) in facts.imports.iter().enumerate() {
                // Browser scripts are analyzed independently with root scopes;
                // their imports must not inherit the server template domain.
                let domain = facts
                    .scopes
                    .iter()
                    .find(|scope| {
                        scope.parent.is_none()
                            && scope.domain != ScopeDomain::Server
                            && scope.span.start <= import.span.start
                            && import.span.end <= scope.span.end
                    })
                    .map_or(ScopeDomain::Server, |scope| scope.domain);
                let resolution = resolve(ModuleRequest {
                    importer: path,
                    domain,
                    source: &import.source,
                    span: import.span.clone(),
                    source_span: import.source_span.clone(),
                    dependency: ModuleDependency::Import(import),
                });
                let dependency = snapshot.dependencies.len();
                if let ModuleResolution::Resolved(target) = &resolution {
                    snapshot
                        .importers
                        .entry(target.clone())
                        .or_default()
                        .push(dependency);
                }
                snapshot.dependencies.push(Dependency {
                    path: path.clone(),
                    index: import_index,
                    domain,
                    resolution,
                });
                index.imports.push(dependency);
                for binding in &import.bindings {
                    index.bindings.insert(binding.declaration, dependency);
                }
            }
            for reference in &facts.references {
                if let Resolution::Resolved(binding) = &reference.resolution
                    && let Some(&dependency) = index.bindings.get(&binding.id)
                    && let ModuleResolution::Resolved(target) =
                        &snapshot.dependencies[dependency].resolution
                {
                    snapshot
                        .references
                        .entry(target.clone())
                        .or_default()
                        .push(ImportUse {
                            dependency,
                            reference: reference.id,
                        });
                }
            }
            for (export_index, export) in facts.exports.iter().enumerate() {
                let Some(source) = &export.source else {
                    continue;
                };
                let domain = facts
                    .scopes
                    .iter()
                    .find(|s| s.parent.is_none())
                    .expect("module root scope")
                    .domain;
                let resolution = resolve(ModuleRequest {
                    importer: path,
                    domain,
                    source,
                    span: export.declaration_span.clone(),
                    source_span: export
                        .source_span
                        .clone()
                        .expect("re-export module literal"),
                    dependency: ModuleDependency::Export(export),
                });
                let edge = snapshot.reexports.len();
                if let ModuleResolution::Resolved(target) = &resolution {
                    snapshot
                        .reexporters
                        .entry(target.clone())
                        .or_default()
                        .push(edge);
                }
                snapshot.reexports.push(ReExportEdge {
                    path: path.clone(),
                    index: export_index,
                    resolution,
                });
                index.reexports.push(edge);
                index.export_edges.insert(export_index, edge);
            }
            snapshot.indexes.insert(path.clone(), index);
        }
        // Build incoming use sets once; unused_exports does one membership test
        // per export, irrespective of the number of consumers.
        let mut uses = Vec::new();
        for dependency in &snapshot.dependencies {
            if let ModuleResolution::Resolved(target) = &dependency.resolution {
                let facts = snapshot.files[&dependency.path]
                    .analysis
                    .as_ref()
                    .as_ref()
                    .expect("indexed facts");
                for binding in &facts.imports[dependency.index].bindings {
                    uses.push((target.clone(), Some(binding.target.specifier.clone())));
                }
            }
        }
        for edge in &snapshot.reexports {
            if let ModuleResolution::Resolved(target) = &edge.resolution {
                let facts = snapshot.files[&edge.path]
                    .analysis
                    .as_ref()
                    .as_ref()
                    .expect("indexed facts");
                uses.push((target.clone(), facts.exports[edge.index].imported.clone()));
            }
        }
        for (target, specifier) in uses {
            let Some(index) = snapshot.indexes.get_mut(&target) else {
                continue;
            };
            match specifier {
                Some(ImportSpecifier::Namespace) => index.used_namespace = true,
                Some(ImportSpecifier::Named(name)) => {
                    index.used_names.insert(name);
                }
                Some(ImportSpecifier::Default) => {
                    index.used_names.insert("default".into());
                }
                None => index.used_star = true,
            }
        }
        snapshot
    }
}
