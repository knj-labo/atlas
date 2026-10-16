use crate::{Binding, DeclarationId, FileFacts, Reference, Resolution};

impl FileFacts {
    /// Find all declarations whose name exactly matches `name` (case-sensitive).
    ///
    /// Results borrow [`Self::declarations`] and are ordered by source span,
    /// then ID for equal spans. Same-name declarations in different scopes or
    /// script domains remain distinct. Import aliases use their local name.
    pub fn declarations_named(&self, name: &str) -> Vec<&Binding> {
        let mut declarations: Vec<_> = self
            .declarations
            .iter()
            .filter(|binding| binding.name == name)
            .collect();
        declarations.sort_by_key(|binding| (binding.span.start, binding.span.end, binding.id));
        declarations
    }

    /// Find the reference whose half-open span contains `offset`.
    ///
    /// The offset is a UTF-8 byte position in the original source, not a
    /// character or UTF-16 index. Any byte within `[start, end)` matches,
    /// including a continuation byte of a multibyte character. Returns `None`
    /// outside reference spans, including at end of file or beyond it.
    ///
    /// The result borrows [`Self::references`] and retains its [`Resolution`],
    /// including builtin, unresolved, and runtime-dependent states. This only
    /// searches recorded references; it does not infer uses from declaration
    /// names, property names, text, or CSS candidates.
    pub fn reference_at(&self, offset: u32) -> Option<&Reference> {
        self.references
            .iter()
            .find(|reference| reference.span.contains(&offset))
    }

    /// Find references definitely resolved to `declaration` in this result.
    ///
    /// Results borrow [`Self::references`] and are ordered by source span,
    /// then ID for equal spans. Both value and type references are included.
    /// Builtin, unresolved, and runtime-dependent references (even those listing
    /// this ID as a candidate) are excluded. An unknown ID returns no matches.
    ///
    /// IDs belong only to the containing analysis result. Obtain a new ID from
    /// the new result after a source update; numeric IDs can be reused and this
    /// method cannot detect an ID supplied from a different result.
    pub fn references_to(&self, declaration: DeclarationId) -> Vec<&Reference> {
        if !self
            .declarations
            .iter()
            .any(|binding| binding.id == declaration)
        {
            return Vec::new();
        }
        let mut references: Vec<_> = self
            .references
            .iter()
            .filter(|reference| {
                matches!(&reference.resolution, Resolution::Resolved(binding) if binding.id == declaration)
            })
            .collect();
        references
            .sort_by_key(|reference| (reference.span.start, reference.span.end, reference.id));
        references
    }
}
