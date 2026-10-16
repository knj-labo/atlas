use crate::{
    AnalysisError, FileFacts, ModuleResolver, ProjectFile, ProjectRevision, ProjectSnapshot,
    RelativeResolver, analyze,
};
use salsa::Setter;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// On-demand, file-level analysis with owned, shared results.
///
/// Available with the `incremental` feature. Callers supply paths and complete
/// source text; this database performs no file I/O or path canonicalization.
/// Paths are compared using [`PathBuf`]'s equality and hashing rules.
///
/// Registration and updates do not run analysis. The next query for a changed
/// file analyzes its entire source. Other files reuse their memoized results.
/// Files may be removed with [`Self::remove_file`]. Internal Salsa inputs remain
/// allocated until the database is dropped;
/// previously returned [`Arc`] values can outlive both updates and the database.
/// Facts IDs remain local to each result, including across source revisions.
///
/// ```
/// use atlas::AnalysisDatabase;
/// use std::sync::Arc;
///
/// let mut db = AnalysisDatabase::new();
/// assert!(db.analyze_file("Button.astro").is_none());
/// db.set_source("Button.astro", "<Button />");
/// let first = db.analyze_file("Button.astro").unwrap();
/// let cached = db.analyze_file("Button.astro").unwrap();
/// assert!(Arc::ptr_eq(&first, &cached));
///
/// db.set_source("Button.astro", "<Updated />");
/// let updated = db.analyze_file("Button.astro").unwrap();
/// assert_eq!(updated.as_ref().as_ref().unwrap().components[0].name, "Updated");
/// drop(db);
/// assert_eq!(first.as_ref().as_ref().unwrap().components[0].name, "Button");
/// ```
pub struct AnalysisDatabase {
    database: Database,
    files: HashMap<PathBuf, SourceFile>,
    resolver: Box<dyn ModuleResolver>,
    revision: ProjectRevision,
    project: OnceLock<Arc<ProjectSnapshot>>,
}

impl Default for AnalysisDatabase {
    fn default() -> Self {
        Self {
            database: Database::default(),
            files: HashMap::new(),
            resolver: Box::new(RelativeResolver),
            revision: ProjectRevision::default(),
            project: OnceLock::new(),
        }
    }
}

impl AnalysisDatabase {
    /// Create an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a path or replace its complete source text without analyzing it.
    /// Re-registering identical text preserves the existing input revision.
    pub fn set_source(&mut self, path: impl Into<PathBuf>, source: impl Into<String>) {
        let path = path.into();
        let source = source.into();
        if let Some(&file) = self.files.get(&path) {
            if file.text(&self.database).as_ref() != source.as_str() {
                file.set_text(&mut self.database).to(source.into());
                self.invalidate_project_cache();
            }
        } else {
            let file = SourceFile::new(&self.database, Arc::<str>::from(source));
            self.files.insert(path, file);
            self.invalidate_project_cache();
        }
    }

    /// Remove a registered path and invalidate project resolution and indexes.
    /// Returns whether the path existed. Earlier returned results remain valid.
    /// Internal Salsa storage is retained until database destruction.
    pub fn remove_file(&mut self, path: impl AsRef<Path>) -> bool {
        if self.files.remove(path.as_ref()).is_none() {
            return false;
        }
        self.invalidate_project_cache();
        true
    }

    /// Replace the module resolver and its immutable settings.
    /// Every call starts a new resolver revision and invalidates the project
    /// snapshot, including previously missing imports. File analyses are reused.
    pub fn set_module_resolver(&mut self, resolver: impl ModuleResolver + 'static) {
        self.resolver = Box::new(resolver);
        self.revision.resolver = self
            .revision
            .resolver
            .checked_add(1)
            .expect("resolver revision overflow");
        self.project.take();
    }

    /// Analyze registered Astro buffers and index direct imports and their uses.
    ///
    /// Unchanged calls reuse the same snapshot. Any source addition, update,
    /// removal, or resolver replacement rebuilds the project index on demand;
    /// unchanged file analyses reuse their existing memos. No files are read
    /// from disk. Syntax errors remain available in each [`ProjectFile`].
    ///
    /// ```
    /// let mut db = atlas::AnalysisDatabase::new();
    /// db.set_source("Button.astro", "<button />");
    /// db.set_source("Page.astro", "---\nimport Button from './Button.astro';\n---\n<Button />");
    /// let project = db.analyze_project();
    /// assert_eq!(project.incoming_imports("Button.astro").len(), 1);
    /// assert_eq!(project.references_to_file("Button.astro").len(), 1);
    /// assert!(db.remove_file("Button.astro"));
    /// assert!(db.analyze_project().references_to_file("Button.astro").is_empty());
    /// assert_eq!(project.references_to_file("Button.astro").len(), 1);
    /// ```
    pub fn analyze_project(&self) -> Arc<ProjectSnapshot> {
        Arc::clone(self.project.get_or_init(|| {
            let files = self
                .files
                .iter()
                .map(|(path, &file)| {
                    (
                        path.clone(),
                        ProjectFile {
                            source: Arc::clone(file.text(&self.database)),
                            analysis: analyze_input(&self.database, file),
                        },
                    )
                })
                .collect();
            Arc::new(ProjectSnapshot::build(
                Some(self.revision),
                files,
                self.resolver.as_ref(),
            ))
        }))
    }

    fn invalidate_project_cache(&mut self) {
        self.revision.sources = self
            .revision
            .sources
            .checked_add(1)
            .expect("source revision overflow");
        self.project.take();
    }

    /// Analyze a registered file, reusing the cached result when possible.
    ///
    /// Returns `None` for an unregistered path. Both `Ok(FileFacts)` and
    /// `Err(AnalysisError)` are memoized with the same contract as [`analyze`].
    /// Successful facts may contain Astro diagnostics with error severity.
    pub fn analyze_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Option<Arc<Result<FileFacts, AnalysisError>>> {
        self.files
            .get(path.as_ref())
            .map(|&file| analyze_input(&self.database, file))
    }
}

#[salsa::db]
#[derive(Clone, Default)]
struct Database {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for Database {}

#[salsa::input]
struct SourceFile {
    #[returns(ref)]
    text: Arc<str>,
}

#[salsa::tracked(returns(clone))]
fn analyze_input(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Arc<Result<FileFacts, AnalysisError>> {
    Arc::new(analyze(file.text(db)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DiagnosticPhase;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counted_database() -> (AnalysisDatabase, Arc<AtomicUsize>) {
        let executions = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&executions);
        let storage = salsa::Storage::new(Some(Box::new(move |event| {
            if matches!(event.kind, salsa::EventKind::WillExecute { .. }) {
                count.fetch_add(1, Ordering::Relaxed);
            }
        })));
        (
            AnalysisDatabase {
                database: Database { storage },
                ..AnalysisDatabase::default()
            },
            executions,
        )
    }

    #[test]
    fn registration_is_lazy_and_queries_are_memoized() {
        let (mut db, executions) = counted_database();
        assert!(db.analyze_file("Missing.astro").is_none());
        db.set_source("A.astro", "<A />");
        assert_eq!(executions.load(Ordering::Relaxed), 0);

        let first = db.analyze_file("A.astro").unwrap();
        assert_eq!(executions.load(Ordering::Relaxed), 1);
        let cached = db.analyze_file("A.astro").unwrap();
        assert!(Arc::ptr_eq(&first, &cached));
        assert_eq!(executions.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn identical_updates_do_not_execute_the_query() {
        let (mut db, executions) = counted_database();
        db.set_source("A.astro", "<A />");
        let first = db.analyze_file("A.astro").unwrap();
        db.set_source("A.astro", String::from("<A />"));
        let cached = db.analyze_file("A.astro").unwrap();
        assert!(Arc::ptr_eq(&first, &cached));
        assert_eq!(executions.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn updating_one_file_preserves_other_memos() {
        let (mut db, executions) = counted_database();
        db.set_source("A.astro", "<A />");
        db.set_source("B.astro", "<B />");
        let first_a = db.analyze_file("A.astro").unwrap();
        let first_b = db.analyze_file("B.astro").unwrap();
        assert_eq!(executions.load(Ordering::Relaxed), 2);

        db.set_source("A.astro", "<Updated />");
        assert_eq!(executions.load(Ordering::Relaxed), 2);
        assert!(Arc::ptr_eq(&first_b, &db.analyze_file("B.astro").unwrap()));
        assert_eq!(executions.load(Ordering::Relaxed), 2);

        let updated = db.analyze_file("A.astro").unwrap();
        assert_eq!(updated.as_ref(), &analyze("<Updated />"));
        assert!(!Arc::ptr_eq(&first_a, &updated));
        assert!(Arc::ptr_eq(&updated, &db.analyze_file("A.astro").unwrap()));
        assert_eq!(executions.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn pending_updates_analyze_only_the_latest_source() {
        let (mut db, executions) = counted_database();
        db.set_source("A.astro", "<A />");
        db.analyze_file("A.astro").unwrap();
        db.set_source("A.astro", "---\nconst =;\n---");
        db.set_source("A.astro", "<Latest />");
        assert_eq!(executions.load(Ordering::Relaxed), 1);
        let latest = db.analyze_file("A.astro").unwrap();
        assert_eq!(latest.as_ref(), &analyze("<Latest />"));
        assert_eq!(executions.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn syntax_errors_are_memoized_and_recover_after_updates() {
        for (phase, invalid) in [
            (
                DiagnosticPhase::Parser,
                include_str!("../tests/fixtures/invalid-frontmatter.astro"),
            ),
            (
                DiagnosticPhase::Semantic,
                include_str!("../tests/spec/regression-semantic-error.astro"),
            ),
            (
                DiagnosticPhase::Css,
                include_str!("../tests/spec/invalid-css.astro"),
            ),
        ] {
            let (mut db, executions) = counted_database();
            db.set_source("A.astro", "<A />");
            assert!(db.analyze_file("A.astro").unwrap().is_ok());

            db.set_source("A.astro", invalid);
            let error = db.analyze_file("A.astro").unwrap();
            assert_eq!(error.as_ref(), &analyze(invalid));
            assert_eq!(
                error.as_ref().as_ref().unwrap_err().diagnostics[0].phase,
                phase
            );
            assert!(Arc::ptr_eq(&error, &db.analyze_file("A.astro").unwrap()));
            db.set_source("A.astro", invalid);
            db.analyze_file("A.astro").unwrap();
            assert_eq!(executions.load(Ordering::Relaxed), 2);

            db.set_source("A.astro", "<Fixed />");
            let fixed = db.analyze_file("A.astro").unwrap();
            assert_eq!(fixed.as_ref(), &analyze("<Fixed />"));
            assert!(fixed.is_ok());
            assert!(error.is_err());
            assert_eq!(executions.load(Ordering::Relaxed), 3);
        }
    }

    #[test]
    fn equal_sources_at_different_paths_have_independent_inputs() {
        let (mut db, executions) = counted_database();
        db.set_source("A.astro", "<Shared />");
        db.set_source("B.astro", "<Shared />");
        let a = db.analyze_file("A.astro").unwrap();
        let b = db.analyze_file("B.astro").unwrap();
        assert_eq!(a, b);
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(executions.load(Ordering::Relaxed), 2);
        db.set_source("A.astro", "<Changed />");
        assert!(Arc::ptr_eq(&b, &db.analyze_file("B.astro").unwrap()));
        assert_eq!(executions.load(Ordering::Relaxed), 2);
    }
}
