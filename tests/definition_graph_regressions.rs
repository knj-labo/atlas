use atlas::{
    DefinitionKind, DefinitionResolution, DefinitionUnresolved, ProjectSnapshot, analyze_project,
};
use std::path::Path;

fn definition<'a>(
    project: &'a ProjectSnapshot,
    path: &str,
    name: &str,
) -> DefinitionResolution<'a> {
    let source = &project.file(path).unwrap().source;
    project
        .definition_at(path, source.rfind(name).unwrap() as u32)
        .unwrap()
}

#[test]
fn missing_local_exports_are_unresolved_including_default_aliases() {
    let project = analyze_project([
        (
            "broken.ts",
            "export { missing, absent as renamed, unknown as default };",
        ),
        (
            "use.ts",
            "import main, { missing, renamed } from './broken.ts'; missing; renamed; main;",
        ),
    ]);
    assert!(project.file("broken.ts").unwrap().analysis.is_ok());
    for name in ["missing", "renamed", "main"] {
        assert!(
            matches!(
                definition(&project, "use.ts", name),
                DefinitionResolution::Unresolved(DefinitionUnresolved::UnresolvedReference)
            ),
            "an undeclared local export must not become a saved expression: {name}"
        );
    }
}

#[test]
fn default_expression_exports_and_local_default_aliases_keep_distinct_targets() {
    let project = analyze_project([
        ("expression.ts", "export default 42;"),
        ("anonymous.ts", "export default () => 42;"),
        ("local.ts", "const value = 42; export { value as default };"),
        (
            "use.ts",
            "import expression from './expression.ts'; import anonymous from './anonymous.ts'; import local from './local.ts'; expression; anonymous; local;",
        ),
    ]);
    for (name, path) in [
        ("expression", "expression.ts"),
        ("anonymous", "anonymous.ts"),
    ] {
        let DefinitionResolution::Resolved(target) = definition(&project, "use.ts", name) else {
            panic!("expected the saved default expression for {name}")
        };
        assert_eq!(target.path, Path::new(path));
        assert!(matches!(target.kind, DefinitionKind::Export(_)));
    }
    let DefinitionResolution::Resolved(target) = definition(&project, "use.ts", "local") else {
        panic!("expected the aliased local declaration")
    };
    assert_eq!(target.path, Path::new("local.ts"));
    assert!(matches!(target.kind, DefinitionKind::Declaration(binding) if binding.name == "value"));
}

#[test]
fn shared_star_cycles_keep_all_targets_from_every_entry_and_query_order() {
    let project = analyze_project([
        (
            "left.ts",
            "export * from './right.ts'; export * from './first.ts';",
        ),
        (
            "right.ts",
            "export * from './left.ts'; export * from './second.ts';",
        ),
        ("first.ts", "export const value = 1;"),
        ("second.ts", "export const value = 2;"),
        (
            "shared.ts",
            "export * from './left.ts'; export * from './right.ts';",
        ),
        (
            "use.ts",
            "import { value as left } from './left.ts'; import { value as right } from './right.ts'; import { value as shared } from './shared.ts'; left; right; shared; left; right; shared;",
        ),
    ]);
    for name in ["left", "right", "shared", "shared", "right", "left"] {
        let DefinitionResolution::Ambiguous(targets) = definition(&project, "use.ts", name) else {
            panic!("every cycle entry must retain both distinct targets: {name}")
        };
        assert_eq!(
            targets.iter().map(|target| target.path).collect::<Vec<_>>(),
            [Path::new("first.ts"), Path::new("second.ts")]
        );
    }
    #[cfg(feature = "serde")]
    {
        let report = project.report();
        let uses = report
            .files
            .iter()
            .find(|file| file.path == "use.ts")
            .unwrap();
        assert_eq!(uses.references.len(), 6);
        for reference in &uses.references {
            let atlas::DefinitionReport::Ambiguous(targets) = &reference.definition else {
                panic!("batch roots must retain all SCC targets: {reference:?}")
            };
            assert_eq!(
                targets
                    .iter()
                    .map(|target| target.key.split('#').next().unwrap())
                    .collect::<Vec<_>>(),
                ["first.ts", "second.ts"]
            );
        }
    }
}

#[test]
fn shared_star_cycles_do_not_hide_failed_branches() {
    let project = analyze_project([
        (
            "left.ts",
            "export * from './right.ts'; export * from './value.ts';",
        ),
        (
            "right.ts",
            "export * from './left.ts'; export * from './broken.ts';",
        ),
        ("value.ts", "export const value = 1;"),
        ("broken.ts", "const =;"),
        (
            "use.ts",
            "import { value as left } from './left.ts'; import { value as right } from './right.ts'; left; right;",
        ),
    ]);
    for name in ["left", "right", "left"] {
        assert_eq!(
            match definition(&project, "use.ts", name) {
                DefinitionResolution::Unresolved(reason) => reason,
                other => panic!("failed cycle branches must keep the result uncertain: {other:?}"),
            },
            DefinitionUnresolved::AnalysisFailed {
                path: "broken.ts".into()
            }
        );
    }
}

#[test]
fn shared_module_export_aliases_keep_distinct_declarations() {
    let project = analyze_project([
        (
            "values.ts",
            "export const first = 1; export const second = 2;",
        ),
        ("left.ts", "export { first as value } from './values.ts';"),
        ("right.ts", "export { second as value } from './values.ts';"),
        (
            "barrel.ts",
            "export * from './left.ts'; export * from './right.ts';",
        ),
        ("use.ts", "import { value } from './barrel.ts'; value;"),
    ]);
    let DefinitionResolution::Ambiguous(targets) = definition(&project, "use.ts", "value") else {
        panic!("different export names in one module must both be traversed")
    };
    assert_eq!(
        targets
            .iter()
            .map(|target| match target.kind {
                DefinitionKind::Declaration(binding) => binding.name.as_str(),
                _ => panic!("expected declarations"),
            })
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn layered_shared_stars_deduplicate_the_original_declaration() {
    // This contains thousands of paths but only 27 modules. Keep the graph
    // bounded so correctness stays quick even without traversal memoization.
    let mut sources = vec![("value.ts".to_owned(), "export const value = 1;".to_owned())];
    for level in (0..12).rev() {
        let next = if level == 11 {
            "export * from './value.ts';".to_owned()
        } else {
            format!(
                "export * from './left{}.ts'; export * from './right{}.ts';",
                level + 1,
                level + 1
            )
        };
        for side in ["left", "right"] {
            sources.push((format!("{side}{level}.ts"), next.clone()));
        }
    }
    sources.push((
        "barrel.ts".into(),
        "export * from './left0.ts'; export * from './right0.ts';".into(),
    ));
    sources.push((
        "use.ts".into(),
        "import { value } from './barrel.ts'; value;".into(),
    ));
    let project = analyze_project(sources);
    let DefinitionResolution::Resolved(target) = definition(&project, "use.ts", "value") else {
        panic!("all star paths reach the same declaration")
    };
    assert_eq!(target.path, Path::new("value.ts"));
    assert!(matches!(target.kind, DefinitionKind::Declaration(binding) if binding.name == "value"));
}
