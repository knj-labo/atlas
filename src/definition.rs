use super::{ProjectFile, ProjectReference, ProjectSnapshot};
use crate::{
    Binding, DeclarationId, Export, ExportLocal, ImportSpecifier, Meaning, ModuleResolution,
    Resolution, ScopeDomain, SymbolId,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
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
}

/// Definition of a recorded reference. Ambiguous candidates are distinct,
/// confirmed targets in deterministic path/span order, never guessed bindings.
#[derive(Debug, Clone)]
pub enum DefinitionResolution<'a> {
    Resolved(Definition<'a>),
    Ambiguous(Vec<Definition<'a>>),
    Unresolved(DefinitionUnresolved),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

// Keys and terminals own only snapshot-local IDs. No cached record borrows the
// snapshot, allowing all entry points and threads to share completed SCCs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Node {
    Export(String),
    Symbol(SymbolId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    file: usize,
    node: Node,
    meaning: Meaning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TargetKind {
    Declaration(DeclarationId),
    Export(usize),
    AstroComponent,
    Module,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Target {
    file: usize,
    kind: TargetKind,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Failure {
    path: PathBuf,
    name: String,
    reason_name: &'static str,
    reason: DefinitionUnresolved,
}

impl DefinitionUnresolved {
    fn name(&self) -> &'static str {
        match self {
            Self::UnresolvedReference => "UnresolvedReference",
            Self::Builtin => "Builtin",
            Self::RuntimeDependent => "RuntimeDependent",
            Self::UnresolvedModule => "UnresolvedModule",
            Self::UnsupportedModule => "UnsupportedModule",
            Self::AmbiguousModule { .. } => "AmbiguousModule",
            Self::UnsupportedSourceType { .. } => "UnsupportedSourceType",
            Self::AnalysisFailed { .. } => "AnalysisFailed",
            Self::MissingExport { .. } => "MissingExport",
            Self::Cycle { .. } => "Cycle",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Completed {
    targets: BTreeSet<Target>,
    blocking: Option<Failure>,
    cycle: Option<Failure>,
    missing: Option<Failure>,
}

fn retain_first<T: Ord>(into: &mut Option<T>, value: T) {
    if into.as_ref().is_none_or(|existing| value < *existing) {
        *into = Some(value);
    }
}

impl Completed {
    fn failed(&mut self, project: &ProjectSnapshot, key: &Key, reason: DefinitionUnresolved) {
        let name = match &key.node {
            Node::Export(name) => name.clone(),
            Node::Symbol(id) => {
                let facts = project.facts(key.file);
                facts.declarations[facts.symbols[id.0].declarations[0].0]
                    .name
                    .clone()
            }
        };
        let slot = match &reason {
            DefinitionUnresolved::Cycle { .. } => &mut self.cycle,
            DefinitionUnresolved::MissingExport { .. } => &mut self.missing,
            _ => &mut self.blocking,
        };
        retain_first(
            slot,
            Failure {
                path: project.file_paths[key.file].clone(),
                name,
                reason_name: reason.name(),
                reason,
            },
        );
    }

    fn merge(&mut self, other: &Self) {
        self.targets.extend(other.targets.iter().copied());
        for (slot, value) in [
            (&mut self.blocking, &other.blocking),
            (&mut self.cycle, &other.cycle),
            (&mut self.missing, &other.missing),
        ] {
            if let Some(value) = value {
                retain_first(slot, value.clone());
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.targets.is_empty()
            && self.blocking.is_none()
            && self.cycle.is_none()
            && self.missing.is_none()
    }

    fn borrowed<'a>(&self, project: &'a ProjectSnapshot) -> DefinitionResolution<'a> {
        if self.targets.len() > 1 {
            let mut targets: Vec<_> = self
                .targets
                .iter()
                .map(|t| project.borrow_target(*t))
                .collect();
            targets.sort_by(|a, b| {
                (a.path, a.span().start, a.span().end).cmp(&(b.path, b.span().start, b.span().end))
            });
            return DefinitionResolution::Ambiguous(targets);
        }
        if let Some(failure) = &self.blocking {
            return DefinitionResolution::Unresolved(failure.reason.clone());
        }
        if let Some(target) = self.targets.first() {
            return DefinitionResolution::Resolved(project.borrow_target(*target));
        }
        let failure = self
            .cycle
            .as_ref()
            .or(self.missing.as_ref())
            .expect("completed search has a terminal");
        DefinitionResolution::Unresolved(failure.reason.clone())
    }
}

#[derive(Default)]
struct Expansion {
    local: Completed,
    edges: HashSet<Key>,
    #[cfg(test)]
    export_comparisons: usize,
}

#[derive(Debug, Default)]
pub(super) struct ResolutionCache {
    completed: HashMap<Key, Arc<Completed>>,
    #[cfg(test)]
    pub(super) expansions: usize,
    #[cfg(test)]
    pub(super) target_insertions: usize,
    #[cfg(test)]
    pub(super) export_comparisons: usize,
}

impl ResolutionCache {
    fn resolve(&mut self, project: &ProjectSnapshot, root: Key) -> Arc<Completed> {
        if let Some(result) = self.completed.get(&root) {
            return Arc::clone(result);
        }
        let mut keys = vec![root.clone()];
        let mut ids = HashMap::from([(root.clone(), 0)]);
        let mut expansions = Vec::new();
        let mut cursor = 0;
        // Expand only nodes absent from the snapshot's finished cache.
        while cursor < keys.len() {
            let expansion = project.expand(&keys[cursor]);
            #[cfg(test)]
            {
                self.expansions += 1;
                self.export_comparisons += expansion.export_comparisons;
            }
            for key in &expansion.edges {
                if !self.completed.contains_key(key) && !ids.contains_key(key) {
                    ids.insert(key.clone(), keys.len());
                    keys.push(key.clone());
                }
            }
            expansions.push(expansion);
            cursor += 1;
        }
        let adjacency: Vec<Vec<usize>> = expansions
            .iter()
            .map(|e| {
                e.edges
                    .iter()
                    .filter_map(|key| ids.get(key).copied())
                    .collect()
            })
            .collect();
        // Iterative Tarjan: SCCs finish in dependency order. The explicit DFS
        // stack also handles arbitrarily long re-export chains without recursion.
        let mut index = vec![None; keys.len()];
        let mut low = vec![0; keys.len()];
        let mut on_stack = vec![false; keys.len()];
        let mut active = Vec::new();
        let mut sequence = 0;
        for start in 0..keys.len() {
            if index[start].is_some() {
                continue;
            }
            index[start] = Some(sequence);
            low[start] = sequence;
            sequence += 1;
            active.push(start);
            on_stack[start] = true;
            let mut dfs = vec![(start, 0)];
            while let Some((node, next)) = dfs.last_mut() {
                let v = *node;
                if let Some(&w) = adjacency[v].get(*next) {
                    *next += 1;
                    if index[w].is_none() {
                        index[w] = Some(sequence);
                        low[w] = sequence;
                        sequence += 1;
                        active.push(w);
                        on_stack[w] = true;
                        dfs.push((w, 0));
                    } else if on_stack[w] {
                        low[v] = low[v].min(index[w].unwrap());
                    }
                    continue;
                }
                dfs.pop();
                if let Some(&(parent, _)) = dfs.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v].unwrap() {
                    let mut group = Vec::new();
                    loop {
                        let w = active.pop().expect("active SCC");
                        on_stack[w] = false;
                        group.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let members: HashSet<_> = group.iter().copied().collect();
                    let cyclic = group.len() > 1 || adjacency[v].contains(&v);
                    let mut result = Completed::default();
                    let mut dependencies = Vec::new();
                    let mut shared = HashSet::new();
                    for &node in &group {
                        result.merge(&expansions[node].local);
                        for edge in &expansions[node].edges {
                            if ids.get(edge).is_some_and(|id| members.contains(id)) {
                                continue;
                            }
                            let completed = &self.completed[edge];
                            if shared.insert(Arc::as_ptr(completed)) {
                                dependencies.push(Arc::clone(completed));
                            }
                        }
                        if cyclic {
                            let key = &keys[node];
                            let name = match &key.node {
                                Node::Export(name) => name.clone(),
                                Node::Symbol(id) => {
                                    let facts = project.facts(key.file);
                                    facts.declarations[facts.symbols[id.0].declarations[0].0]
                                        .name
                                        .clone()
                                }
                            };
                            result.failed(
                                project,
                                key,
                                DefinitionUnresolved::Cycle {
                                    path: project.file_paths[key.file].clone(),
                                    name,
                                },
                            );
                        }
                    }
                    let result = if result.is_empty() && dependencies.len() == 1 {
                        // Alias chains share the same immutable candidate set.
                        dependencies.pop().unwrap()
                    } else {
                        for dependency in dependencies {
                            #[cfg(test)]
                            {
                                self.target_insertions += dependency.targets.len();
                            }
                            result.merge(&dependency);
                        }
                        Arc::new(result)
                    };
                    for node in group {
                        self.completed
                            .insert(keys[node].clone(), Arc::clone(&result));
                    }
                }
            }
        }
        Arc::clone(&self.completed[&root])
    }
}

/// Borrowing facade; the shared snapshot cache owns every completed result.
pub(crate) struct DefinitionResolver<'a> {
    project: &'a ProjectSnapshot,
    #[cfg(test)]
    pub(crate) searches: usize,
}

impl<'a> DefinitionResolver<'a> {
    pub(crate) fn new(project: &'a ProjectSnapshot) -> Self {
        Self {
            project,
            #[cfg(test)]
            searches: 0,
        }
    }

    fn resolve(&mut self, key: Key) -> DefinitionResolution<'a> {
        let result = {
            let mut cache = self
                .project
                .definitions
                .lock()
                .expect("definition cache lock");
            #[cfg(test)]
            {
                self.searches += usize::from(!cache.completed.contains_key(&key));
            }
            cache.resolve(self.project, key)
        };
        // Convert IDs to borrowed records only after releasing the mutex.
        result.borrowed(self.project)
    }

    pub(crate) fn reference(
        &mut self,
        reference: ProjectReference<'a>,
    ) -> DefinitionResolution<'a> {
        match &reference.reference.resolution {
            Resolution::Resolved(binding) => self.resolve(Key {
                file: self.project.indexes[reference.path].file_id,
                node: Node::Symbol(binding.symbol),
                meaning: reference.reference.meaning,
            }),
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

    pub(crate) fn export(&mut self, path: &'a Path, name: &str) -> DefinitionResolution<'a> {
        self.resolve(Key {
            file: self.project.indexes[path].file_id,
            node: Node::Export(name.into()),
            meaning: Meaning::Value,
        })
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

    fn facts(&self, file: usize) -> &crate::FileFacts {
        self.files[&self.file_paths[file]]
            .analysis
            .as_ref()
            .as_ref()
            .expect("analyzed graph node")
    }

    fn borrow_target(&self, target: Target) -> Definition<'_> {
        let path = self.file_paths[target.file].as_path();
        let file = &self.files[path];
        Definition {
            path,
            kind: match target.kind {
                TargetKind::Declaration(id) => {
                    DefinitionKind::Declaration(&self.facts(target.file).declarations[id.0])
                }
                TargetKind::Export(id) => {
                    DefinitionKind::Export(&self.facts(target.file).exports[id])
                }
                TargetKind::AstroComponent => DefinitionKind::AstroComponent(file),
                TargetKind::Module => DefinitionKind::Module(file),
            },
        }
    }

    fn in_module(
        &self,
        key: &Key,
        resolution: &ModuleResolution,
        specifier: &ImportSpecifier,
        expansion: &mut Expansion,
    ) {
        let target = match resolution {
            ModuleResolution::Resolved(path) => path,
            ModuleResolution::Unresolved => {
                expansion
                    .local
                    .failed(self, key, DefinitionUnresolved::UnresolvedModule);
                return;
            }
            ModuleResolution::Unsupported => {
                expansion
                    .local
                    .failed(self, key, DefinitionUnresolved::UnsupportedModule);
                return;
            }
            ModuleResolution::Ambiguous(candidates) => {
                expansion.local.failed(
                    self,
                    key,
                    DefinitionUnresolved::AmbiguousModule {
                        candidates: candidates.clone(),
                    },
                );
                return;
            }
        };
        let file = &self.files[target];
        if let Err(error) = file.analysis.as_ref() {
            expansion.local.failed(
                self,
                key,
                if error
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_deref() == Some("atlas/unsupported-source-type"))
                {
                    DefinitionUnresolved::UnsupportedSourceType {
                        path: target.clone(),
                    }
                } else {
                    DefinitionUnresolved::AnalysisFailed {
                        path: target.clone(),
                    }
                },
            );
            return;
        }
        let file = self.indexes[target].file_id;
        let name = match specifier {
            ImportSpecifier::Namespace => {
                expansion.local.targets.insert(Target {
                    file,
                    kind: TargetKind::Module,
                });
                return;
            }
            ImportSpecifier::Default => "default",
            ImportSpecifier::Named(name) => name,
        };
        expansion.edges.insert(Key {
            file,
            node: Node::Export(name.into()),
            meaning: key.meaning,
        });
    }

    fn expand(&self, key: &Key) -> Expansion {
        let mut expansion = Expansion::default();
        let facts = self.facts(key.file);
        let path = &self.file_paths[key.file];
        let index = &self.indexes[path];
        match &key.node {
            Node::Symbol(id) => {
                if let Some(binding) = facts.symbols[id.0].declaration(facts, key.meaning) {
                    if let Some(import) = &binding.import {
                        if import.type_only && key.meaning == Meaning::Value {
                            expansion.local.failed(
                                self,
                                key,
                                DefinitionUnresolved::UnresolvedReference,
                            );
                        } else {
                            let dependency = index.bindings[&binding.id];
                            self.in_module(
                                key,
                                &self.dependencies[dependency].resolution,
                                &import.specifier,
                                &mut expansion,
                            );
                        }
                    } else {
                        expansion.local.targets.insert(Target {
                            file: key.file,
                            kind: TargetKind::Declaration(binding.id),
                        });
                    }
                } else {
                    expansion
                        .local
                        .failed(self, key, DefinitionUnresolved::UnresolvedReference);
                }
            }
            Node::Export(name) => {
                if name == "default"
                    && facts
                        .scopes
                        .iter()
                        .any(|s| s.parent.is_none() && s.domain == ScopeDomain::Server)
                {
                    if matches!(key.meaning, Meaning::Value | Meaning::ValueAsType) {
                        expansion.local.targets.insert(Target {
                            file: key.file,
                            kind: TargetKind::AstroComponent,
                        });
                    } else {
                        expansion.local.failed(
                            self,
                            key,
                            DefinitionUnresolved::UnresolvedReference,
                        );
                    }
                } else if let Some(explicit) = index.exports.get(name) {
                    // Decide explicit-name priority before filtering meanings.
                    let mut applicable = false;
                    for &id in explicit {
                        #[cfg(test)]
                        {
                            expansion.export_comparisons += 1;
                        }
                        let export = &facts.exports[id];
                        if !export.meanings.contains(&key.meaning) {
                            continue;
                        }
                        applicable = true;
                        if export.source.is_some() {
                            self.in_module(
                                key,
                                &self.reexports[index.export_edges[&id]].resolution,
                                export.imported.as_ref().expect("named re-export"),
                                &mut expansion,
                            );
                        } else if let Some(target) = export.target {
                            let symbol = match target {
                                ExportLocal::Declaration(id) => facts.declarations[id.0].symbol,
                                ExportLocal::Symbol(id) => id,
                            };
                            expansion.edges.insert(Key {
                                file: key.file,
                                node: Node::Symbol(symbol),
                                meaning: key.meaning,
                            });
                        } else if export.default_expression {
                            expansion.local.targets.insert(Target {
                                file: key.file,
                                kind: TargetKind::Export(id),
                            });
                        } else {
                            expansion.local.failed(
                                self,
                                key,
                                DefinitionUnresolved::UnresolvedReference,
                            );
                        }
                    }
                    if !applicable {
                        expansion.local.failed(
                            self,
                            key,
                            DefinitionUnresolved::UnresolvedReference,
                        );
                    }
                } else {
                    let mut type_only_stars = false;
                    if name != "default" {
                        for &id in &index.stars {
                            let export = &facts.exports[id];
                            if export.type_only && key.meaning == Meaning::Value {
                                type_only_stars = true;
                            } else {
                                self.in_module(
                                    key,
                                    &self.reexports[index.export_edges[&id]].resolution,
                                    &ImportSpecifier::Named(name.clone()),
                                    &mut expansion,
                                );
                            }
                        }
                    }
                    if expansion.edges.is_empty() && expansion.local.is_empty() {
                        expansion.local.failed(
                            self,
                            key,
                            if type_only_stars {
                                DefinitionUnresolved::UnresolvedReference
                            } else {
                                DefinitionUnresolved::MissingExport {
                                    path: path.clone(),
                                    name: name.clone(),
                                }
                            },
                        );
                    }
                }
            }
        }
        expansion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(project: &ProjectSnapshot, path: &str, name: &str) -> Key {
        Key {
            file: project.indexes[Path::new(path)].file_id,
            node: Node::Export(name.into()),
            meaning: Meaning::Value,
        }
    }

    #[test]
    fn shared_star_graph_expands_each_export_once_across_all_queries() {
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
        let mut cache = project.definitions.lock().unwrap();
        for level in (0..depth).rev() {
            for prefix in ["a", "b"] {
                let result = cache.resolve(
                    &project,
                    key(&project, &format!("{prefix}{level}.ts"), "value"),
                );
                assert_eq!(result.targets.len(), 1);
            }
        }
        assert_eq!(cache.expansions, 2 * depth + 2);
        assert_eq!(cache.export_comparisons, 1);
        assert_eq!(cache.target_insertions, 0);
    }

    #[test]
    fn long_chains_share_completed_sets_and_never_reexpand_suffixes() {
        let depth = 5_000;
        let mut sources = vec![("value.ts".to_owned(), "export const value = 1;".to_owned())];
        for i in 0..depth {
            let target = if i == 0 {
                "value.ts".into()
            } else {
                format!("m{}.ts", i - 1)
            };
            sources.push((
                format!("m{i}.ts"),
                format!("export {{ value }} from './{target}';"),
            ));
        }
        let project = crate::analyze_project(sources);
        let mut cache = project.definitions.lock().unwrap();
        let root = cache.resolve(&project, key(&project, "m4999.ts", "value"));
        for i in 0..depth {
            let result = cache.resolve(&project, key(&project, &format!("m{i}.ts"), "value"));
            assert!(
                Arc::ptr_eq(&root, &result),
                "alias suffix must reuse its immutable terminal set"
            );
        }
        assert_eq!(cache.expansions, depth + 2);
        assert_eq!(cache.export_comparisons, depth + 1);
        assert_eq!(cache.target_insertions, 0);
    }

    #[test]
    fn distinct_export_names_and_many_consumers_have_linear_indexed_work() {
        let count = 2_000;
        let values = (0..count)
            .map(|i| format!("export const v{i} = {i};"))
            .collect::<String>();
        let consumers = (0..count).map(|i| {
            (
                format!("use{i}.ts"),
                "import { v0 } from './value.ts'; v0;".to_owned(),
            )
        });
        let mut sources = vec![("value.ts".to_owned(), values)];
        sources.extend(consumers);
        let project = crate::analyze_project(sources);
        let used = &project.indexes[Path::new("value.ts")];
        assert_eq!(used.used_names.len(), 1);
        assert_eq!(project.unused_exports().len(), count - 1);
        {
            let mut cache = project.definitions.lock().unwrap();
            for i in 0..count {
                cache.resolve(&project, key(&project, "value.ts", &format!("v{i}")));
            }
            assert_eq!(cache.expansions, 2 * count);
            assert_eq!(cache.export_comparisons, count);
        }
        for i in 0..count {
            let source = &project.file(format!("use{i}.ts")).unwrap().source;
            assert!(matches!(
                project.definition_at(format!("use{i}.ts"), source.rfind("v0").unwrap() as u32),
                Some(DefinitionResolution::Resolved(_))
            ));
        }
        let cache = project.definitions.lock().unwrap();
        assert_eq!(cache.expansions, 3 * count);
        assert_eq!(cache.export_comparisons, count);
    }

    #[test]
    fn component_index_and_reports_share_the_same_completed_cache() {
        let project = crate::analyze_project([
            ("Button.astro", "<button />"),
            (
                "Page.astro",
                "---\nimport Button from './Button.astro';\n---\n<Button /><Button />",
            ),
        ]);
        assert_eq!(project.component_uses_of("Button.astro").len(), 2);
        let index = project.component_uses.get().unwrap() as *const _;
        let expansions = project.definitions.lock().unwrap().expansions;
        for _ in 0..8 {
            assert_eq!(project.component_uses_of("Button.astro").len(), 2);
            let source = &project.file("Page.astro").unwrap().source;
            project.definition_at("Page.astro", source.rfind("Button").unwrap() as u32);
            #[cfg(feature = "serde")]
            {
                project.report();
            }
        }
        assert_eq!(index, project.component_uses.get().unwrap() as *const _);
        assert_eq!(project.definitions.lock().unwrap().expansions, expansions);
    }
}
