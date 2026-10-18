use std::{error::Error, fmt, ops::Range};

macro_rules! id_type {
    ($($name:ident),+ $(,)?) => {$ (
        /// An index valid only within the containing analysis result.
        /// Obtain IDs again after a source update; they are not stable across results.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
        pub struct $name(pub usize);
    )+};
}
id_type!(
    DeclarationId,
    ScopeId,
    ReferenceId,
    ElementId,
    BoundaryId,
    StyleId,
    CssRuleId,
    CssDeclarationId
);

/// Owned facts; every span is a UTF-8 byte interval in the original source.
///
/// Use [`Self::declarations_named`], [`Self::reference_at`], and
/// [`Self::references_to`] for read-only, file-local searches borrowing these
/// records. Multiple matches are returned in source order without changing IDs
/// or the order of the stored records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct FileFacts {
    pub imports: Vec<Import>,
    pub components: Vec<ComponentUse>,
    pub elements: Vec<Element>,
    pub declarations: Vec<Binding>,
    pub exports: Vec<Export>,
    pub scopes: Vec<Scope>,
    pub references: Vec<Reference>,
    pub scripts: Vec<Script>,
    pub styles: Vec<Style>,
    pub builtin_uses: Vec<BuiltinUse>,
    pub diagnostics: Vec<Diagnostic>,
    /// Explicit server/browser boundaries. Rust-only until the report contract
    /// includes transfer analysis; the existing serialized facts stay unchanged.
    #[cfg_attr(any(feature = "serde", test), serde(skip))]
    pub boundaries: Vec<crate::BoundaryFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct SourceText {
    pub span: Range<u32>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Import {
    pub span: Range<u32>,
    pub source: String,
    pub source_span: Range<u32>,
    pub bindings: Vec<ImportBinding>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct ImportBinding {
    pub name: String,
    pub span: Range<u32>,
    pub target: ImportTarget,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct ImportTarget {
    pub source: String,
    pub specifier: ImportSpecifier,
    pub type_only: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ImportSpecifier {
    Default,
    Named(String),
    Namespace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct ComponentUse {
    pub element: ElementId,
    pub name: String,
    pub span: Range<u32>,
    pub name_span: Range<u32>,
    pub closing_name_span: Option<Range<u32>>,
    pub reference: Reference,
    pub directives: Vec<Directive>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Element {
    pub id: ElementId,
    pub parent: Option<ElementId>,
    pub kind: ElementKind,
    pub name: String,
    pub span: Range<u32>,
    pub opening_span: Range<u32>,
    pub name_span: Option<Range<u32>>,
    pub closing_span: Option<Range<u32>>,
    pub reference: Option<ReferenceId>,
    pub attributes: Vec<Attribute>,
    pub directives: Vec<Directive>,
    pub text: Vec<SourceText>,
    pub slot: Option<SlotAssignment>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ElementKind {
    Html,
    CustomElement,
    Component,
    Dynamic,
    Fragment,
    Slot,
    SelfComponent,
    Script,
    Style,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct SlotAssignment {
    pub name: Option<String>,
    pub span: Range<u32>,
    pub dynamic: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Attribute {
    pub name: Option<String>,
    pub span: Range<u32>,
    pub name_span: Option<Range<u32>>,
    pub value: Option<DirectiveValue>,
    pub literal: Option<Literal>,
    pub spread: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum Literal {
    String(String),
    Number(String),
    Boolean(bool),
    Null,
    Array(Vec<Literal>),
    Object(Vec<(String, Literal)>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Directive {
    pub name: String,
    pub span: Range<u32>,
    pub name_span: Range<u32>,
    pub value: Option<DirectiveValue>,
    pub status: ValidationStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct DirectiveValue {
    pub raw: String,
    pub span: Range<u32>,
    pub kind: DirectiveValueKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum DirectiveValueKind {
    String,
    Expression,
    Element,
    Fragment,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ValidationStatus {
    Valid,
    Invalid,
    Dynamic,
    RequiresProjectContext,
    UnknownDirective,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub span: Range<u32>,
    pub domain: ScopeDomain,
    pub kind: ScopeKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ScopeDomain {
    Server,
    BrowserModule,
    BrowserClassic,
    InjectedScript,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ScopeKind {
    Root,
    Function,
    Block,
    Class,
    Type,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Reference {
    pub id: ReferenceId,
    pub name: String,
    pub span: Range<u32>,
    pub scope: ScopeId,
    pub kind: ReferenceKind,
    pub resolution: Resolution,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ReferenceKind {
    /// `this` or `super`, whose receiver is determined at runtime.
    Receiver,
    Value,
    Type,
    ValueAndType,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum Resolution {
    Resolved(Binding),
    Builtin(Builtin),
    Unresolved,
    RuntimeDependent {
        reason: String,
        candidates: Vec<DeclarationId>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum Builtin {
    Astro,
    Fragment,
    AstroSelf,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Binding {
    pub id: DeclarationId,
    pub scope: ScopeId,
    pub name: String,
    pub span: Range<u32>,
    pub kind: DeclarationKind,
    pub import: Option<ImportTarget>,
    pub injection: Option<InjectionOrigin>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum DeclarationKind {
    Import,
    Variable,
    Function,
    Class,
    Parameter,
    Type,
    Injected,
    Other,
}
/// An explicit frontmatter export. The component itself is the implicit
/// default export of every `.astro` file and is not listed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Export {
    pub name: String,
    pub span: Range<u32>,
    /// The local declaration; `None` for re-exports.
    pub local: Option<DeclarationId>,
    /// The module specifier of a re-export.
    pub source: Option<String>,
    /// The name taken from `source` by a re-export; `None` for local exports
    /// and for `export * from`, which is recorded with the name `*`.
    pub imported: Option<ImportSpecifier>,
    pub type_only: bool,
    pub role: ExportRole,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ExportRole {
    /// Read by Astro: `getStaticPaths`, `prerender`, or `partial`.
    Astro,
    /// The component's `Props` type.
    Props,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct InjectionOrigin {
    pub key_span: Range<u32>,
    pub value: SourceText,
    pub references: Vec<ReferenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct BuiltinUse {
    pub builtin: Builtin,
    pub member: Option<String>,
    pub span: Range<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Script {
    pub element: ElementId,
    pub span: Range<u32>,
    pub content: SourceText,
    pub mode: ScriptMode,
    /// Whether this script is eligible for compiler bundling.
    pub bundled: bool,
    pub language: ScriptLanguage,
    pub source: Option<DirectiveValue>,
    pub references: Vec<Reference>,
    pub injected: Vec<DeclarationId>,
    pub dynamic_injection: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ScriptMode {
    Processed,
    InlineClassic,
    InlineModule,
    External,
    Data,
    Dynamic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum ScriptLanguage {
    TypeScript,
    JavaScript,
    Raw,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Style {
    pub id: StyleId,
    pub element: ElementId,
    pub span: Range<u32>,
    pub content: SourceText,
    pub mode: StyleMode,
    pub language: String,
    pub inline_attribute: bool,
    pub dynamic_injection: bool,
    pub css: Option<CssFacts>,
    pub injected: Vec<CssInjection>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum StyleMode {
    Scoped,
    Global,
    Inline,
    Dynamic,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssInjection {
    pub name: String,
    pub origin: InjectionOrigin,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssFacts {
    pub rules: Vec<CssRule>,
    pub declarations: Vec<CssDeclaration>,
    pub references: Vec<CssReference>,
    pub definitions: Vec<CssDefinition>,
    pub dependencies: Vec<CssDependency>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssRule {
    pub id: CssRuleId,
    pub parent: Option<CssRuleId>,
    pub span: Range<u32>,
    pub prelude: SourceText,
    pub kind: CssRuleKind,
    pub selectors: Vec<CssSelector>,
    pub conditions: Vec<SourceText>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum CssRuleKind {
    Style,
    Media,
    Supports,
    Container,
    Layer,
    Scope,
    Keyframes,
    Keyframe,
    Property,
    AtRule(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssSelector {
    pub span: Range<u32>,
    pub raw: String,
    pub classes: Vec<CssName>,
    pub ids: Vec<CssName>,
    pub global: bool,
    pub nesting: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssName {
    pub name: String,
    pub span: Range<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssDeclaration {
    pub id: CssDeclarationId,
    pub rule: Option<CssRuleId>,
    pub span: Range<u32>,
    pub name: String,
    pub name_span: Range<u32>,
    pub value: SourceText,
    pub important: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssDefinition {
    pub name: String,
    pub span: Range<u32>,
    pub kind: CssDefinitionKind,
    pub declaration: Option<CssDeclarationId>,
    pub conditions: Vec<SourceText>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum CssDefinitionKind {
    CustomProperty,
    RegisteredProperty,
    Keyframes,
    Injected,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssReference {
    pub name: String,
    pub span: Range<u32>,
    pub kind: CssReferenceKind,
    pub fallback: Option<SourceText>,
    pub candidates: Vec<CssCandidate>,
    pub runtime_dependent: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum CssReferenceKind {
    CustomProperty,
    Animation,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssCandidate {
    pub style: StyleId,
    pub definition: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct CssDependency {
    pub kind: CssDependencyKind,
    pub value: String,
    pub span: Range<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum CssDependencyKind {
    Import,
    Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct AnalysisError {
    pub diagnostics: Vec<Diagnostic>,
    pub parser_panicked: bool,
}
impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.diagnostics.first() {
            Some(d) => write!(f, "Astro analysis failed: {}", d.message),
            None => f.write_str("Astro analysis failed"),
        }
    }
}
impl Error for AnalysisError {}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct Diagnostic {
    pub phase: DiagnosticPhase,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum DiagnosticPhase {
    Parser,
    Semantic,
    Astro,
    Css,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub enum Severity {
    Error,
    Warning,
    Information,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "serde", test), derive(serde::Serialize))]
pub struct DiagnosticLabel {
    pub span: Range<u32>,
    pub message: Option<String>,
}
