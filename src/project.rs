use crate::{
    AnalysisError, DeclarationId, DeclarationKind, FileFacts, Import, ModuleRequest,
    ModuleResolution, ModuleResolver, ProjectSources, Reference, ReferenceId, RelativeResolver,
    Resolution, ScopeDomain, analyze,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Analyze Astro sources and index their direct imports without a cache.
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

/// Analyze Astro sources without a cache, using a caller-supplied resolver.
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
            let analysis = Arc::new(analyze(&source));
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

/// A borrowed import, its source file, and its module resolution.
#[derive(Debug, Clone, Copy)]
pub struct ProjectImport<'a> {
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
    bindings: HashMap<DeclarationId, usize>,
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
/// This indexes static import declarations and their lexical uses, not exports,
/// re-exports, dynamic imports, script `src`, CSS, or transitive dependencies.
#[derive(Debug)]
pub struct ProjectSnapshot {
    revision: Option<ProjectRevision>,
    files: BTreeMap<PathBuf, ProjectFile>,
    dependencies: Vec<Dependency>,
    indexes: BTreeMap<PathBuf, FileIndex>,
    importers: BTreeMap<PathBuf, Vec<usize>>,
    references: BTreeMap<PathBuf, Vec<ImportUse>>,
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

    /// All static imports from a file, including unused and side-effect imports.
    /// Unknown or failed files return no matches; inspect [`Self::file`] for errors.
    pub fn imports_from(&self, path: impl AsRef<Path>) -> Vec<ProjectImport<'_>> {
        self.indexes
            .get(path.as_ref())
            .into_iter()
            .flat_map(|index| &index.imports)
            .map(|&index| self.import(index))
            .collect()
    }

    /// Imports whose module resolved to this exact registered path key.
    pub fn imports_to(&self, path: impl AsRef<Path>) -> Vec<ProjectImport<'_>> {
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
    /// are available through [`Self::imports_to`] instead.
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
        let import = match &reference.resolution {
            Resolution::Resolved(binding) => self.indexes[path]
                .bindings
                .get(&binding.id)
                .map(|&index| self.import(index)),
            _ => None,
        };
        Some(ProjectReference {
            path,
            reference,
            import,
        })
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
        };
        for (path, file) in &snapshot.files {
            let Ok(facts) = file.analysis.as_ref() else {
                continue;
            };
            let mut index = FileIndex::default();
            let bindings: HashMap<_, _> = facts
                .declarations
                .iter()
                .filter(|binding| binding.kind == DeclarationKind::Import)
                .map(|binding| ((binding.span.start, binding.span.end), binding.id))
                .collect();
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
                let mut resolution = resolver.resolve(
                    ModuleRequest {
                        importer: path,
                        import,
                        domain,
                    },
                    &context,
                );
                if let ModuleResolution::Resolved(target) = &resolution
                    && !context.contains(target)
                {
                    resolution = ModuleResolution::Unresolved;
                }
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
                    if let Some(&id) = bindings.get(&(binding.span.start, binding.span.end)) {
                        index.bindings.insert(id, dependency);
                    }
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
            snapshot.indexes.insert(path.clone(), index);
        }
        snapshot
    }
}
