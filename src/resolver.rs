use crate::{Export, Import, ScopeDomain};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// Result of resolving an import's module, not an exported declaration or type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ModuleResolution {
    /// An exact registered path key. The target may have an analysis error.
    Resolved(PathBuf),
    /// No registered target was found.
    Unresolved,
    /// This resolver does not handle the specifier or import context.
    Unsupported,
    /// Alternative module paths reported by the resolver. These are diagnostic
    /// candidates, not validated or indexed as resolved imports.
    Ambiguous(Vec<PathBuf>),
}

/// Original import/re-export, source ranges and execution domain for a resolver.
#[derive(Debug, Clone)]
pub struct ModuleRequest<'a> {
    pub importer: &'a Path,
    pub domain: ScopeDomain,
    pub source: &'a str,
    /// Full import or export statement in the original UTF-8 source.
    pub span: Range<u32>,
    /// Original module string literal, including quotes.
    pub source_span: Range<u32>,
    pub dependency: ModuleDependency<'a>,
}

/// The original source fact, including specifiers and type-only modifiers.
#[derive(Debug, Clone, Copy)]
pub enum ModuleDependency<'a> {
    Import(&'a Import),
    Export(&'a Export),
}

/// Immutable source buffers belonging to the project being resolved.
///
/// Paths use the caller's keys with [`PathBuf`] equality. No filesystem access,
/// canonicalization, case folding, or symlink resolution is performed.
pub struct ProjectSources<'a> {
    sources: &'a BTreeMap<PathBuf, Arc<str>>,
    normalized: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl<'a> ProjectSources<'a> {
    pub(crate) fn new(sources: &'a BTreeMap<PathBuf, Arc<str>>) -> Self {
        let mut normalized: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for path in sources.keys() {
            normalized
                .entry(normalize(path))
                .or_default()
                .push(path.clone());
        }
        Self {
            sources,
            normalized,
        }
    }

    /// Registered path keys in path order, including files with syntax errors.
    pub fn paths(&self) -> impl ExactSizeIterator<Item = &Path> {
        self.sources.keys().map(PathBuf::as_path)
    }

    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.sources.contains_key(path.as_ref())
    }

    pub fn source(&self, path: impl AsRef<Path>) -> Option<&str> {
        self.sources.get(path.as_ref()).map(AsRef::as_ref)
    }
}

/// Adapter boundary for module resolution, independent of type analysis.
///
/// Implementations must be deterministic functions of their immutable settings,
/// request, and supplied source snapshot. Do not read changing external state:
/// supply its contents as immutable settings for each analysis. With the optional
/// `incremental` API, replace the resolver using
/// `AnalysisDatabase::set_module_resolver` whenever its settings change.
/// Snapshots retain resolver output; querying them never invokes the resolver.
/// Return exact supplied path keys for `Resolved`; snapshot construction converts a
/// result naming an absent key to `Unresolved`.
///
/// ```
/// use atlas::{ModuleRequest, ModuleResolution, ModuleResolver, ProjectSources, RelativeResolver};
/// struct WithAlias;
/// impl ModuleResolver for WithAlias {
///     fn resolve(&self, request: ModuleRequest<'_>, sources: &ProjectSources<'_>) -> ModuleResolution {
///         if request.source == "@ui/button" {
///             ModuleResolution::Resolved("src/Button.astro".into())
///         } else {
///             RelativeResolver.resolve(request, sources)
///         }
///     }
/// }
/// let project = atlas::analyze_project_with_resolver([
///     ("Page.astro", "---\nimport Button from '@ui/button';\n---\n<Button />"),
///     ("src/Button.astro", "<button />"),
/// ], &WithAlias);
/// assert_eq!(project.references_to_file("src/Button.astro").len(), 1);
/// ```
pub trait ModuleResolver: Send + Sync {
    fn resolve(&self, request: ModuleRequest<'_>, sources: &ProjectSources<'_>)
    -> ModuleResolution;
}

/// Resolves explicit `./` and `../` paths against registered virtual files.
///
/// Dot/parent components and repeated separators are normalized for matching;
/// registered keys are preserved. Multiple keys with the same normalized path
/// are ambiguous.
/// No extensions or directory indexes are inferred. Bare specifiers, absolute
/// paths, URLs, query/hash suffixes, backslash paths, and directory forms ending
/// in `/`, `/.`, or `/..` are unsupported. Replace
/// this resolver to implement aliases or a different module-resolution policy.
#[derive(Debug, Default, Clone, Copy)]
pub struct RelativeResolver;

impl ModuleResolver for RelativeResolver {
    fn resolve(
        &self,
        request: ModuleRequest<'_>,
        sources: &ProjectSources<'_>,
    ) -> ModuleResolution {
        let specifier = request.source;
        if !(specifier.starts_with("./") || specifier.starts_with("../"))
            || specifier.contains(['?', '#', '\\', '\0'])
            || specifier.ends_with('/')
            || specifier.ends_with("/.")
            || specifier.ends_with("/..")
        {
            return ModuleResolution::Unsupported;
        }
        let parent = request.importer.parent().unwrap_or(Path::new(""));
        let target = normalize(&parent.join(specifier));
        match sources.normalized.get(&target).map(Vec::as_slice) {
            Some([path]) => ModuleResolution::Resolved(path.clone()),
            Some(paths) => ModuleResolution::Ambiguous(paths.to_vec()),
            None => ModuleResolution::Unresolved,
        }
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push(component);
                }
            }
            _ => result.push(component),
        }
    }
    result
}
