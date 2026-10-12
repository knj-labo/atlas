use std::{error::Error, fmt, ops::Range};

/// Facts own all strings and contain no references into Oxc's allocator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileFacts {
    /// Static imports, including side-effect imports and imports inside scripts.
    pub imports: Vec<Import>,
    /// Template elements with an identifier root, in source order.
    pub components: Vec<ComponentUse>,
    /// Parsed browser scripts and their identifier references, in source order.
    pub scripts: Vec<Script>,
}

/// Every range is a UTF-8 byte interval `[start, end)` in the input source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub span: Range<u32>,
    pub source: String,
    pub source_span: Range<u32>,
    /// Empty for side-effect imports and empty named imports.
    pub bindings: Vec<ImportBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBinding {
    pub name: String,
    pub span: Range<u32>,
    pub target: ImportTarget,
}

/// Describes the import syntax, without resolving the module on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportTarget {
    pub source: String,
    pub specifier: ImportSpecifier,
    pub type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSpecifier {
    Default,
    Named(String),
    Namespace,
}

/// A paired opening/closing tag represents one use of a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentUse {
    /// Full tag name, such as `Counter` or `UI.Forms.Counter`.
    pub name: String,
    pub span: Range<u32>,
    pub name_span: Range<u32>,
    pub closing_name_span: Option<Range<u32>>,
    /// The identifier root only; member properties are not resolved.
    pub reference: Reference,
    /// Attributes with a colon in their name, preserving order and duplicates.
    pub directives: Vec<Directive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub name: String,
    pub span: Range<u32>,
    pub name_span: Range<u32>,
    pub value: Option<DirectiveValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveValue {
    /// Original source text, including quotes or braces.
    pub raw: String,
    pub span: Range<u32>,
    pub kind: DirectiveValueKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveValueKind {
    String,
    Expression,
    Element,
    Fragment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// Includes the script tags.
    pub span: Range<u32>,
    pub references: Vec<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub span: Range<u32>,
    pub resolution: Resolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved(Binding),
    /// Valid syntax for which Oxc found no matching declaration.
    Unresolved,
}

/// Binding identity is its declaration span within this analysis result.
/// No Oxc SymbolId is exposed or assumed to be stable across analyses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub span: Range<u32>,
    pub import: Option<ImportTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisError {
    pub diagnostics: Vec<Diagnostic>,
    /// Oxc's parser reported early termination; semantic analysis was skipped.
    pub parser_panicked: bool,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.diagnostics.first() {
            Some(diagnostic) => write!(f, "Astro analysis failed: {}", diagnostic.message),
            None => f.write_str("Astro analysis failed"),
        }
    }
}

impl Error for AnalysisError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub phase: DiagnosticPhase,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticPhase {
    Parser,
    Semantic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub span: Range<u32>,
    pub message: Option<String>,
}
