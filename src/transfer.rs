//! Owned boundary facts and conservative, method-specific transfer rules.

use crate::{
    BoundaryId, DeclarationId, Directive, ElementId, FileFacts, ReferenceId, ScopeDomain,
    ScriptMode, SourceText, ValidationStatus,
};
use std::ops::Range;

/// A boundary describes a value handoff, independently of lexical scope.
/// All spans and raw text refer to the original Astro source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryFact {
    pub id: BoundaryId,
    pub element: ElementId,
    pub kind: BoundaryKind,
    pub method: TransferMethod,
    pub span: Range<u32>,
    /// Where the input expressions are evaluated. Their lexical scopes remain
    /// available through `BoundaryInput::references` and `FileFacts::scopes`.
    pub evaluation: ScopeDomain,
    /// `None` when the script's execution mode has not been established.
    pub destination: Option<ScopeDomain>,
    /// Copies of the existing directive validation results, without reinterpretation.
    pub directives: Vec<Directive>,
    /// Syntactic inputs, including duplicates. Conditions describe uncertainty
    /// about the effective input set and do not select a winning value.
    pub inputs: Vec<BoundaryInput>,
    pub conditions: Vec<TransferCondition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    ScriptDefineVars,
    ClientProps,
}

/// Contracts pinned to Astro 7.0.6. This does not select a renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMethod {
    /// `stringifyForScript`: JSON with script-safe escaping.
    JsonScript,
    /// Astro's tagged `serializeProps` representation, including BigInt.
    AstroProps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryInput {
    /// `None` for an input set or key that cannot be enumerated statically.
    pub name: Option<String>,
    pub key_span: Option<Range<u32>>,
    pub value: TransferValue,
    /// References in the input expression itself, in source order. References
    /// followed through constant aliases are recorded in `value.origins`.
    pub references: Vec<ReferenceId>,
}

/// Collected while the parser is alive; assessment needs no AST or source buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferValue {
    pub source: SourceText,
    pub kind: TransferValueKind,
    pub origins: Vec<ConstantOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferValueKind {
    Null,
    String(String),
    Boolean(bool),
    /// A finite numeric literal, optionally signed; stored without float equality.
    FiniteNumber(String),
    BigInt(String),
    Array(Vec<TransferValue>),
    Object(Vec<(String, TransferValue)>),
    Function,
    Unknown(TransferUnknownReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstantOrigin {
    pub declaration: DeclarationId,
    pub reference: ReferenceId,
    pub initializer: SourceText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferUnknownReason {
    MutableBinding,
    AggregateBinding,
    NonPrimitiveBinding,
    ExternalReference,
    UntrackedBinding,
    UncertainInitialization,
    CyclicAlias,
    CallResult,
    DynamicKey,
    Spread,
    DuplicateKey,
    Accessor,
    NonPlainObject,
    ArrayHole,
    NonFiniteNumber,
    InputSet,
    UnsupportedExpression,
    AnalysisLimit,
}

/// A condition outside value-format support, with original-source evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferCondition {
    pub kind: TransferConditionKind,
    pub span: Range<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferConditionKind {
    DirectiveValidation(ValidationStatus),
    /// More than one directive selects the handoff on this element.
    DuplicateDirective,
    DuplicateInput(String),
    DynamicInputSet,
    UnanalyzedScript(ScriptMode),
    /// Spreads or duplicate attributes can change the script's execution mode.
    UncertainScriptAttributes,
    InvalidInjectedName,
    /// Requires project/renderer information, even for `client:only`.
    RendererAvailability,
    /// The caller must decide whether publishing the data is permitted.
    DataDisclosure,
    /// Evaluation order, side effects and relocation require separate analysis.
    CodeMovementSafety,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferAssessment {
    pub boundary: BoundaryId,
    pub method: TransferMethod,
    /// One result per recorded input, with matching indices and order.
    pub inputs: Vec<InputTransferAssessment>,
    /// These remain unverified even when every value has format support.
    pub assumptions: Vec<TransferCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTransferAssessment {
    pub input: usize,
    pub status: TransferStatus,
    /// Value-format evidence; the containing assessment's assumptions also apply.
    pub reasons: Vec<TransferReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    /// The collected value is handled by the specified transfer method. This
    /// does not establish renderer availability, disclosure permission, execution
    /// of the boundary, or safety of moving/removing code.
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferReason {
    pub kind: TransferReasonKind,
    pub span: Range<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferReasonKind {
    Primitive,
    StaticArray,
    PlainObject,
    BigIntEncoding,
    BigIntInJson,
    /// Functions cannot be preserved as values, including when JSON omits them
    /// or substitutes null instead of throwing.
    FunctionValue,
    ConstantInitializer,
    UnknownValue(TransferUnknownReason),
}

impl FileFacts {
    /// Assess an explicit boundary using only owned facts. Invalid IDs return
    /// `None`; IDs belong to this analysis result and must be obtained again
    /// after updates. No expressions are evaluated and no I/O is performed.
    ///
    /// ```
    /// let facts = atlas::analyze("<script define:vars={{ count: 1, large: 1n }}></script>")?;
    /// let result = facts.assess_transfer(facts.boundaries[0].id).unwrap();
    /// assert_eq!(result.inputs[0].status, atlas::TransferStatus::Supported);
    /// assert_eq!(result.inputs[1].status, atlas::TransferStatus::Unsupported);
    /// assert!(facts.assess_transfer(atlas::BoundaryId(usize::MAX)).is_none());
    /// # Ok::<(), atlas::AnalysisError>(())
    /// ```
    pub fn assess_transfer(&self, id: BoundaryId) -> Option<TransferAssessment> {
        let boundary = self.boundaries.get(id.0).filter(|b| b.id == id)?;
        let inputs = boundary
            .inputs
            .iter()
            .enumerate()
            .map(|(input, value)| {
                let (status, reasons) = assess_value(&value.value, boundary.method);
                InputTransferAssessment {
                    input,
                    status,
                    reasons,
                }
            })
            .collect();
        let mut assumptions = boundary.conditions.clone();
        if boundary.kind == BoundaryKind::ClientProps {
            assumptions.push(TransferCondition {
                kind: TransferConditionKind::RendererAvailability,
                span: boundary.span.clone(),
            });
        }
        for kind in [
            TransferConditionKind::DataDisclosure,
            TransferConditionKind::CodeMovementSafety,
        ] {
            assumptions.push(TransferCondition {
                kind,
                span: boundary.span.clone(),
            });
        }
        Some(TransferAssessment {
            boundary: id,
            method: boundary.method,
            inputs,
            assumptions,
        })
    }
}

fn assess_value(
    value: &TransferValue,
    method: TransferMethod,
) -> (TransferStatus, Vec<TransferReason>) {
    use TransferReasonKind as R;
    use TransferStatus as S;
    use TransferValueKind as V;
    let (status, kind, children): (_, _, Vec<&TransferValue>) = match &value.kind {
        V::Null | V::String(_) | V::Boolean(_) | V::FiniteNumber(_) => {
            (S::Supported, R::Primitive, Vec::new())
        }
        V::BigInt(_) if method == TransferMethod::AstroProps => {
            (S::Supported, R::BigIntEncoding, Vec::new())
        }
        V::BigInt(_) => (S::Unsupported, R::BigIntInJson, Vec::new()),
        V::Function => (S::Unsupported, R::FunctionValue, Vec::new()),
        V::Unknown(reason) => (S::Unknown, R::UnknownValue(*reason), Vec::new()),
        V::Array(items) => (S::Supported, R::StaticArray, items.iter().collect()),
        V::Object(properties) => (
            S::Supported,
            R::PlainObject,
            properties.iter().map(|(_, v)| v).collect(),
        ),
    };
    let mut reasons = vec![TransferReason {
        kind,
        span: value.source.span.clone(),
    }];
    reasons.extend(value.origins.iter().map(|origin| TransferReason {
        kind: R::ConstantInitializer,
        span: origin.initializer.span.clone(),
    }));
    let mut status = status;
    for child in children {
        let (child_status, child_reasons) = assess_value(child, method);
        // A definite loss of a value is conclusive. Otherwise missing evidence
        // keeps the aggregate unknown, including when other children are supported.
        status = match (status, child_status) {
            (S::Unsupported, _) | (_, S::Unsupported) => S::Unsupported,
            (S::Unknown, _) | (_, S::Unknown) => S::Unknown,
            _ => S::Supported,
        };
        reasons.extend(child_reasons);
    }
    (status, reasons)
}
