use crate::{facts::*, syntax::diagnostic};

pub(crate) fn validate(facts: &mut FileFacts) {
    let mut diagnostics = Vec::new();
    for element in &mut facts.elements {
        let component = matches!(
            element.kind,
            ElementKind::Component | ElementKind::Dynamic | ElementKind::SelfComponent
        );
        for directive in &mut element.directives {
            let attribute = element
                .attributes
                .iter()
                .find(|a| a.span == directive.span)
                .unwrap();
            let literal = attribute.literal.as_ref();
            let known = matches!(
                directive.name.as_str(),
                "class:list"
                    | "set:html"
                    | "set:text"
                    | "client:load"
                    | "client:idle"
                    | "client:visible"
                    | "client:media"
                    | "client:only"
                    | "server:defer"
                    | "is:global"
                    | "is:inline"
                    | "is:raw"
                    | "define:vars"
                    | "transition:name"
                    | "transition:animate"
                    | "transition:persist"
                    | "transition:persist-props"
            );
            directive.status = if !known {
                ValidationStatus::UnknownDirective
            } else if directive.value.is_some() && literal.is_none() {
                ValidationStatus::Dynamic
            } else {
                ValidationStatus::Valid
            };
            let mut error = None;
            match directive.name.as_str() {
                name if name.starts_with("client:") => {
                    if !component {
                        error = Some((
                            "astro/client-directive-target",
                            "client directives require an imported framework component",
                        ));
                    } else if element.kind == ElementKind::Dynamic {
                        error = Some((
                            "astro/dynamic-hydration",
                            "client directives do not support dynamic tags",
                        ));
                    } else {
                        directive.status = ValidationStatus::RequiresProjectContext;
                    }
                    if name == "client:media"
                        && (directive.value.is_none()
                            || matches!(literal,Some(l) if !matches!(l,Literal::String(_))))
                    {
                        error = Some((
                            "astro/client-media-value",
                            "client:media requires a string media query",
                        ));
                    }
                }
                "server:defer" if !component => {
                    error = Some((
                        "astro/server-directive-target",
                        "server:defer requires a component",
                    ))
                }
                "is:global" if element.kind != ElementKind::Style => {
                    error = Some((
                        "astro/style-directive-target",
                        "is:global applies to style elements",
                    ))
                }
                "define:vars"
                    if !matches!(element.kind, ElementKind::Script | ElementKind::Style) =>
                {
                    error = Some((
                        "astro/define-vars-target",
                        "define:vars applies to literal script or style elements",
                    ))
                }
                "define:vars" if matches!(literal,Some(l) if !matches!(l,Literal::Object(_))) => {
                    error = Some(("astro/define-vars-value", "define:vars requires an object"))
                }
                "transition:name" if matches!(literal,Some(l) if !matches!(l,Literal::String(_))) => {
                    error = Some((
                        "astro/transition-name-value",
                        "transition:name requires a string",
                    ))
                }
                _ => {}
            }
            if let Some((code, message)) = error {
                directive.status = ValidationStatus::Invalid;
                diagnostics.push(diagnostic(
                    DiagnosticPhase::Astro,
                    Severity::Error,
                    code,
                    message,
                    directive.span.clone(),
                ));
            }
        }
        if let Some(slot) = &element.slot
            && slot.dynamic
        {
            diagnostics.push(diagnostic(
                DiagnosticPhase::Astro,
                Severity::Information,
                "astro/dynamic-slot",
                "slot name depends on a runtime value",
                slot.span.clone(),
            ));
        }
        if element.kind == ElementKind::Slot
            && let Some(a) = element
                .attributes
                .iter()
                .find(|a| a.name.as_deref() == Some("name"))
            && !matches!(a.literal, Some(Literal::String(_)))
        {
            diagnostics.push(diagnostic(
                DiagnosticPhase::Astro,
                Severity::Information,
                "astro/dynamic-slot",
                "slot name depends on a runtime value",
                a.span.clone(),
            ));
        }
    }
    const MEMBERS: &[&str] = &[
        "props",
        "params",
        "url",
        "site",
        "clientAddress",
        "isPrerendered",
        "generator",
        "request",
        "response",
        "redirect",
        "rewrite",
        "originPathname",
        "locals",
        "preferredLocale",
        "preferredLocaleList",
        "currentLocale",
        "getActionResult",
        "callAction",
        "routePattern",
        "cookies",
        "session",
        "csp",
        "cache",
        "logger",
        "slots",
        "self",
    ];
    for usage in &facts.builtin_uses {
        if let Some(member) = &usage.member
            && !MEMBERS.contains(&member.split('.').next().unwrap())
        {
            diagnostics.push(diagnostic(
                DiagnosticPhase::Astro,
                Severity::Warning,
                "astro/unknown-api-member",
                format!("Unknown Astro API member {member:?}"),
                usage.span.clone(),
            ));
        }
    }
    facts.diagnostics.extend(diagnostics);
    for component in &mut facts.components {
        component.directives = facts.elements[component.element.0].directives.clone();
    }
}
