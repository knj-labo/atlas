use crate::{AnalysisError, FileFacts, analyze};
use salsa::Setter;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// On-demand, file-level analysis with owned, shared results.
///
/// Available with the `incremental` feature. Callers supply paths and complete
/// source text; this database performs no file I/O or path canonicalization.
/// Paths are compared using [`PathBuf`]'s equality and hashing rules.
///
/// Registration and updates do not run analysis. The next query for a changed
/// file analyzes its entire source. Other files reuse their memoized results.
/// Inputs and cached results are retained until the database is dropped;
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
#[derive(Default)]
pub struct AnalysisDatabase {
    database: Database,
    files: HashMap<PathBuf, SourceFile>,
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
            if file.text(&self.database) != source.as_str() {
                file.set_text(&mut self.database).to(source);
            }
        } else {
            let file = SourceFile::new(&self.database, source);
            self.files.insert(path, file);
        }
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
    #[returns(deref)]
    text: String,
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
                files: HashMap::new(),
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
