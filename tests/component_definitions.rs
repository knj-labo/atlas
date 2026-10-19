use atlas::{DefinitionKind, DefinitionResolution, ProjectSnapshot, analyze_project};
use std::path::Path;

fn component_names(project: &ProjectSnapshot, path: &str) -> Vec<String> {
    project
        .component_uses_of(path)
        .into_iter()
        .map(|usage| usage.component.name.clone())
        .collect()
}

#[test]
fn external_default_reexports_attribute_uses_to_the_component() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        ("barrel.ts", "export { default } from './Card.astro';"),
        ("chain.mjs", "export { default } from './barrel.ts';"),
        (
            "Page.astro",
            "---\nimport Card from './barrel.ts';\nimport Other from './chain.mjs';\n---\n<Card title=\"a\" /><Other />",
        ),
    ]);

    let uses = project.component_uses_of("Card.astro");
    assert_eq!(component_names(&project, "Card.astro"), ["Card", "Other"]);
    assert_eq!(uses[0].path, Path::new("Page.astro"));
    assert_eq!(uses[0].import.import.source, "./barrel.ts");
    assert_eq!(uses[1].import.import.source, "./chain.mjs");
    assert_eq!(uses[0].element.attributes[0].name.as_deref(), Some("title"));
    assert!(project.component_uses_of("barrel.ts").is_empty());
    assert!(project.component_uses_of("chain.mjs").is_empty());
}

#[test]
fn local_import_bindings_reexport_components_with_named_and_default_aliases() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        (
            "barrel.ts",
            "import Local from './Card.astro'; export { Local as Card, Local as default };",
        ),
        (
            "Page.astro",
            "---\nimport DefaultCard, { Card as NamedCard } from './barrel.ts';\n---\n<NamedCard /><DefaultCard />",
        ),
    ]);

    assert_eq!(
        component_names(&project, "Card.astro"),
        ["NamedCard", "DefaultCard"]
    );
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[test]
fn imported_default_expressions_forward_to_the_original_component() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        (
            "barrel.ts",
            "import Card from './Card.astro'; export default Card;",
        ),
        (
            "Page.astro",
            "---\nimport Card from './barrel.ts';\n---\n<Card />",
        ),
    ]);

    assert_eq!(component_names(&project, "Card.astro"), ["Card"]);
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[test]
fn external_component_declarations_and_anonymous_exports_remain_queryable() {
    let project = analyze_project([
        (
            "Named.tsx",
            "export default function Named() { return <p />; }",
        ),
        ("Anonymous.jsx", "export default () => <p />;"),
        (
            "Page.astro",
            "---\nimport Named from './Named.tsx';\nimport Anonymous from './Anonymous.jsx';\n---\n<Named /><Anonymous />",
        ),
    ]);

    assert_eq!(component_names(&project, "Named.tsx"), ["Named"]);
    assert_eq!(component_names(&project, "Anonymous.jsx"), ["Anonymous"]);
}

#[test]
fn explicit_component_exports_override_competing_star_exports() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        ("Other.astro", "<div />"),
        ("card.ts", "export { default as Card } from './Card.astro';"),
        (
            "other.ts",
            "export { default as Card } from './Other.astro';",
        ),
        (
            "barrel.ts",
            "export * from './other.ts'; export { Card } from './card.ts';",
        ),
        (
            "Page.astro",
            "---\nimport { Card } from './barrel.ts';\n---\n<Card />",
        ),
    ]);

    assert_eq!(component_names(&project, "Card.astro"), ["Card"]);
    assert!(project.component_uses_of("Other.astro").is_empty());
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[test]
fn ambiguous_star_exports_do_not_claim_either_component() {
    let page = "---\nimport { Card } from './barrel.ts';\n---\n<Card />";
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        ("Other.astro", "<div />"),
        ("card.ts", "export { default as Card } from './Card.astro';"),
        (
            "other.ts",
            "export { default as Card } from './Other.astro';",
        ),
        (
            "barrel.ts",
            "export * from './card.ts'; export * from './other.ts';",
        ),
        ("Page.astro", page),
    ]);

    let definition = project
        .definition_at("Page.astro", page.rfind("Card").unwrap() as u32)
        .unwrap();
    assert!(
        matches!(definition, DefinitionResolution::Ambiguous(ref targets) if targets.len() == 2)
    );
    assert!(project.component_uses_of("Card.astro").is_empty());
    assert!(project.component_uses_of("Other.astro").is_empty());
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[test]
fn duplicate_paths_to_one_component_count_each_element_once() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        ("card.ts", "export { default as Card } from './Card.astro';"),
        ("left.ts", "export * from './card.ts';"),
        (
            "right.ts",
            "import { Card as Local } from './card.ts'; export { Local as Card };",
        ),
        (
            "barrel.ts",
            "export * from './left.ts'; export * from './right.ts';",
        ),
        (
            "Page.astro",
            "---\nimport { Card } from './barrel.ts';\n---\n<Card /><Card />",
        ),
    ]);

    let uses = project.component_uses_of("Card.astro");
    assert_eq!(uses.len(), 2);
    assert!(uses[0].component.span.start < uses[1].component.span.start);
    assert_eq!(component_names(&project, "Card.astro"), ["Card", "Card"]);
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[test]
fn namespace_roots_stay_modules_while_static_component_members_are_followed() {
    let page = "---\nimport * as UI from './barrel.ts';\nimport * as Direct from './Card.astro';\n---\n<UI.Card /><UI /><UI.Missing /><UI.Card.Nested /><Direct.Card />";
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        (
            "barrel.ts",
            "export { default as Card } from './Card.astro';",
        ),
        ("Page.astro", page),
    ]);

    let definition = project
        .definition_at("Page.astro", page.find("<UI.Card").unwrap() as u32 + 1)
        .unwrap();
    assert!(matches!(definition, DefinitionResolution::Resolved(target)
        if target.path == Path::new("barrel.ts")
            && matches!(target.kind, DefinitionKind::Module(_))));
    assert_eq!(
        component_names(&project, "Card.astro"),
        ["UI.Card", "Direct.Card"]
    );
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[test]
fn namespace_reexports_follow_static_members_without_guessing_missing_astro_exports() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        (
            "parts.ts",
            "export { default as Card } from './Card.astro';",
        ),
        (
            "barrel.ts",
            "export * as Parts from './parts.ts'; export * as Astro from './Card.astro';",
        ),
        (
            "Page.astro",
            "---\nimport * as UI from './barrel.ts';\n---\n<UI.Parts.Card /><UI.Parts.Missing /><UI.Parts.Card.Nested /><UI.Astro.Missing />",
        ),
    ]);

    assert_eq!(component_names(&project, "Card.astro"), ["UI.Parts.Card"]);
    assert!(project.component_uses_of("barrel.ts").is_empty());
    assert!(project.component_uses_of("parts.ts").is_empty());
}

#[test]
fn component_reexports_respect_lexical_shadowing_and_script_domains() {
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        ("barrel.ts", "export { default } from './Card.astro';"),
        (
            "Page.astro",
            "---\nimport Card from './barrel.ts';\n---\n<Card />{[1].map((Card) => <Card />)}\n<script>import Card from './barrel.ts'; Card;</script>\n<script>Card;</script>",
        ),
    ]);

    assert_eq!(component_names(&project, "Card.astro"), ["Card"]);
    assert!(project.component_uses_of("barrel.ts").is_empty());
}

#[cfg(feature = "serde")]
#[test]
fn reports_attribute_external_barrel_component_uses_to_the_component_file() {
    use atlas::{DefinitionReport, DefinitionTargetKind};

    let page = "---\nimport Card from './barrel.ts';\n---\n<Card title=\"x\" client:visible slot=\"aside\" />";
    let project = analyze_project([
        ("Card.astro", "<slot />"),
        ("barrel.ts", "export { default } from './Card.astro';"),
        ("Page.astro", page),
    ]);
    let report = project.report();
    let card = report
        .files
        .iter()
        .find(|file| file.path == "Card.astro")
        .unwrap();
    assert_eq!(card.component_uses.len(), 1);
    let usage = &card.component_uses[0];
    assert_eq!(usage.name, "Card");
    assert_eq!(usage.props, ["title"]);
    assert_eq!(usage.directives, ["client:visible"]);
    assert_eq!(usage.slot.as_deref(), Some("aside"));
    assert_eq!(
        usage.key,
        format!("Page.astro#{}-{}", page.find("<Card").unwrap(), page.len())
    );
    let barrel = report
        .files
        .iter()
        .find(|file| file.path == "barrel.ts")
        .unwrap();
    assert!(barrel.component_uses.is_empty());
    let page = report
        .files
        .iter()
        .find(|file| file.path == "Page.astro")
        .unwrap();
    assert!(
        matches!(&page.references[0].definition, DefinitionReport::Resolved(target)
        if target.key == "Card.astro#0-8" && target.kind == DefinitionTargetKind::AstroComponent)
    );
}
