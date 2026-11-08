//! Narrow acceptance of implicit external-module augmentation dependencies.
use crate::common::{fixture, project, stub_packages};
use crate::run::{check, json, text};
use easement::commands::Outcome;
use std::fs;

const POLICY: &str = "version: 1\ncomponents:\n  platform: { in: src/platform }\n  app: { in: src/app }\n  server: { in: src/server }\nrules:\n  platform: { may_depend_on: [], must_not_reach: [server] }\n  app: { may_depend_on: [server] }\n";
const ACCEPT: &str = "accept_augmentations:\n  - consumers: [platform]\n    module: router-lib\n    declaration: src/app/router.ts\n    because: routing registration is intentional\n";

#[test]
fn augmentation_acceptance_preserves_other_paths_and_rejudges_cached_indexes() {
    let (_tmp, root) = fixture("augmentations");
    let default = check(&root);
    assert_eq!(default.outcome(), Outcome::Failed, "{}", text(&default));
    assert!(text(&default).contains("platform -> app"));
    assert!(text(&default).contains("platform reaches server"));
    for wrong in [
        ACCEPT.replace("[platform]", "[app]"),
        ACCEPT.replace("module: router-lib", "module: other-lib"),
        ACCEPT.replace(
            "declaration: src/app/router.ts",
            "declaration: src/server/secret.ts",
        ),
    ] {
        fs::write(root.join("easement.yml"), format!("{POLICY}{wrong}")).unwrap();
        assert_eq!(check(&root).outcome(), Outcome::Failed);
    }
    fs::write(root.join("easement.yml"), format!("{POLICY}{ACCEPT}")).unwrap();
    let accepted = check(&root);
    assert_eq!(accepted.outcome(), Outcome::Passed, "{}", text(&accepted));
    assert_eq!(
        json(&accepted)["stats"]["accepted_augmentations"][0]["module"],
        "router-lib"
    );
    assert!(text(&accepted).contains("routing registration is intentional"));
    crate::run::pinned(|| {
        let lines = text(&accepted)
            .lines()
            .filter(|line| line.starts_with("Accepted augmentation:"))
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("accepted_augmentation_text", lines);
        insta::assert_json_snapshot!(
            "accepted_augmentation",
            json(&accepted)["stats"]["accepted_augmentations"]
        );
    });
    for source in [
        "export { default as Link } from 'router-lib'; export { app } from '../app/router';\n",
        "export { app } from '../app/router';\nexport { default as Link } from 'router-lib';\n",
        "export { default as Link } from 'router-lib';\nexport { app } from '../app/router';\n",
        "export { secret } from '../server/secret';\n",
    ] {
        fs::write(root.join("src/platform/link.ts"), source).unwrap();
        assert_eq!(check(&root).outcome(), Outcome::Failed);
    }
    fs::write(
        root.join("src/platform/link.ts"),
        "export { default as Link } from 'router-lib';\n",
    )
    .unwrap();
    fs::write(root.join("easement.yml"), POLICY).unwrap();
    assert_eq!(check(&root).outcome(), Outcome::Failed);
}

#[test]
fn multiple_declarations_and_symbol_reach_keep_explicit_and_unrelated_paths() {
    let (_tmp, root) = project(&[
        ("easement.yml", POLICY),
        (
            "package.json",
            r#"{"name":"augmentation-test","version":"1.0.0"}"#,
        ),
        (
            "src/platform/link.ts",
            "export { default as Link } from 'router-lib';\n",
        ),
        (
            "src/app/router.ts",
            "import 'router-lib';\nimport { secret } from '../server/secret';\ndeclare module 'router-lib' { interface Register { router: typeof secret } }\nexport const app = 1;\n",
        ),
        (
            "src/app/navigation.ts",
            "import 'router-lib';\ndeclare module 'router-lib' { interface StaticData { title: string } }\n",
        ),
        ("src/server/secret.ts", "export const secret = 1;\n"),
    ]);
    stub_packages(&root, &["router-lib"]);
    for reach in ["files", "symbols"] {
        let policy = format!("{POLICY}reach: {reach}\n{ACCEPT}");
        fs::write(root.join("easement.yml"), &policy).unwrap();
        assert_eq!(
            check(&root).outcome(),
            Outcome::Failed,
            "another augmentation stays forbidden"
        );
        let policy = format!(
            "{policy}  - consumers: [platform]\n    module: router-lib\n    declaration: src/app/navigation.ts\n    because: static route metadata is intentional\n"
        );
        fs::write(root.join("easement.yml"), &policy).unwrap();
        let accepted = check(&root);
        assert_eq!(accepted.outcome(), Outcome::Passed, "{}", text(&accepted));
        assert_eq!(
            json(&accepted)["stats"]["accepted_augmentations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        fs::write(root.join("src/platform/link.ts"), "export { default as Link } from 'router-lib';\nexport type { Register } from 'router-lib';\n").unwrap();
        assert_eq!(
            check(&root).outcome(),
            Outcome::Failed,
            "explicit augmentation symbol stays a dependency"
        );
        fs::write(
            root.join("src/platform/link.ts"),
            "export { default as Link } from 'router-lib';\nexport { app } from '../app/router';\n",
        )
        .unwrap();
        let direct = check(&root);
        assert_eq!(direct.outcome(), Outcome::Failed);
        assert!(
            text(&direct).contains("platform reaches server"),
            "{}",
            text(&direct)
        );
        fs::write(
            root.join("src/platform/link.ts"),
            "export { default as Link } from 'router-lib';\n",
        )
        .unwrap();
    }
}

#[test]
fn accepted_metadata_does_not_hide_an_unrelated_transitive_path_or_symbol_ban() {
    let policy = POLICY
        .replace(
            "  server: { in: src/server }",
            "  server: { in: src/server }\n  util: { in: src/util }",
        )
        .replace("may_depend_on: []", "may_depend_on: [util]")
        .replace(
            "  app: { may_depend_on: [server] }",
            "  app: { may_depend_on: [server] }\n  util: { may_depend_on: [server] }",
        );
    let (_tmp, root) = project(&[
        ("easement.yml", &format!("{policy}{ACCEPT}")),
        (
            "package.json",
            r#"{"name":"augmentation-test","version":"1.0.0"}"#,
        ),
        (
            "src/platform/link.ts",
            "export { default as Link } from 'router-lib';\nexport { bridge } from '../util/bridge';\n",
        ),
        (
            "src/app/router.ts",
            "import 'router-lib';\ndeclare module 'router-lib' { interface Register { router: string } }\nexport const app = 1;\n",
        ),
        (
            "src/util/bridge.ts",
            "import { secret } from '../server/secret';\nexport const bridge = secret;\n",
        ),
        ("src/server/secret.ts", "export const secret = 1;\n"),
    ]);
    stub_packages(&root, &["router-lib"]);
    for reach in ["files", "symbols"] {
        fs::write(
            root.join("easement.yml"),
            format!("{policy}{ALL}reach: {reach}\n"),
        )
        .unwrap();
        let rejected = check(&root);
        assert!(
            text(&rejected).contains("platform reaches server through util"),
            "{}",
            text(&rejected)
        );
    }
    fs::write(
        root.join("src/platform/link.ts"),
        "export { default as Link } from 'router-lib';\nexport { app } from '../app/router';\n",
    )
    .unwrap();
    fs::write(
        root.join("easement.yml"),
        format!("{POLICY}{ALL}forbid_symbol: [src/app/router.app]\n"),
    )
    .unwrap();
    assert!(text(&check(&root)).contains("[symbol]"));
}

#[test]
fn augmentation_acceptances_require_exact_selectors_and_a_reason() {
    let (_tmp, root) = fixture("augmentations");
    for bad in [
        ACCEPT.replace("[platform]", "[]"),
        ACCEPT.replace("[platform]", "null"),
        ACCEPT.replace("[platform]", "platform"),
        ACCEPT.replace("[platform]", "['']"),
        ACCEPT.replace("src/app/router.ts", "null"),
        ACCEPT.replace("src/app/router.ts", "''"),
        ACCEPT.replace("src/app/router.ts", "[]"),
        ACCEPT.replace("[platform]", "[typo]"),
        ACCEPT.replace("routing registration is intentional", "''"),
        ACCEPT.replace("module: router-lib", "module: './router-lib'"),
        ACCEPT.replace("src/app/router.ts", "src/app/*.ts"),
        ACCEPT.replace("src/app/router.ts", "../router.ts"),
    ] {
        fs::write(root.join("easement.yml"), format!("{POLICY}{bad}")).unwrap();
        let result = crate::run::try_check_with(&root, |_| {});
        assert!(crate::run::error(result).contains("accept_augmentations"));
    }
}

const ALL: &str = "accept_augmentations:\n  - module: router-lib\n    because: routing registration is intentional\n";

#[test]
fn module_acceptance_covers_new_consumers_and_declarations_and_rejudges_cache() {
    let (_tmp, root) = fixture("augmentations");
    let policy = POLICY.replace(
        "  app: { in: src/app }",
        "  app: { in: src/app }\n  ui: { in: src/ui }",
    ) + "  ui: { may_depend_on: [], must_not_reach: [server] }\n";
    crate::common::write_all(
        &root,
        &[
            (
                "src/ui/link.ts",
                "export { default as Link } from 'router-lib';\n",
            ),
            (
                "src/app/static.ts",
                "import 'router-lib';\ndeclare module 'router-lib' { interface StaticData { title: string } }\n",
            ),
        ],
    );
    for reach in ["files", "symbols"] {
        let base = format!("{policy}reach: {reach}\n");
        fs::write(root.join("easement.yml"), &base).unwrap();
        assert_eq!(check(&root).outcome(), Outcome::Failed);
        fs::write(root.join("easement.yml"), format!("{base}{ALL}")).unwrap();
        let accepted = check(&root);
        assert_eq!(accepted.outcome(), Outcome::Passed, "{}", text(&accepted));
        assert_eq!(json(&accepted)["stats"]["from_cache"], 1);
        let relationships = json(&accepted)["stats"]["accepted_augmentations"].clone();
        for consumer in ["src/platform/link.ts", "src/ui/link.ts"] {
            for declaration in ["src/app/router.ts", "src/app/static.ts"] {
                assert!(
                    relationships
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["file"] == consumer && r["declaration"] == declaration)
                );
            }
        }
        crate::run::pinned(|| {
            insta::assert_json_snapshot!("project_wide_augmentations", relationships)
        });
        for selector in [
            "    consumers: [platform]\n",
            "    declaration: src/app/router.ts\n",
        ] {
            fs::write(root.join("easement.yml"), format!("{base}{ALL}{selector}")).unwrap();
            assert_eq!(check(&root).outcome(), Outcome::Failed);
        }
        fs::write(root.join("easement.yml"), &base).unwrap();
        let rejected = check(&root);
        assert_eq!(rejected.outcome(), Outcome::Failed);
        assert_eq!(json(&rejected)["stats"]["from_cache"], 1);
    }
    // Introducing a new component and declaration needs no new acceptance entry.
    let policy = policy.replace(
        "  app: { in: src/app }",
        "  app: { in: src/app }\n  widgets: { in: src/widgets }",
    ) + "  widgets: { may_depend_on: [], must_not_reach: [server] }\n";
    crate::common::write_all(
        &root,
        &[
            ("easement.yml", &format!("{policy}{ALL}")),
            (
                "src/widgets/link.ts",
                "export { default as Link } from 'router-lib';\n",
            ),
            (
                "src/app/generated.ts",
                "import 'router-lib';\ndeclare module 'router-lib' { interface FileRoutes { home: string } }\n",
            ),
        ],
    );
    let accepted = check(&root);
    assert_eq!(accepted.outcome(), Outcome::Passed, "{}", text(&accepted));
    assert!(
        json(&accepted)["stats"]["accepted_augmentations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["file"] == "src/widgets/link.ts"
                && r["declaration"] == "src/app/generated.ts")
    );
}

#[test]
fn project_wide_acceptance_preserves_exact_modules_direct_imports_symbols_and_reach() {
    let (_tmp, root) = fixture("augmentations");
    stub_packages(&root, &["router-lib/sub"]);
    for reach in ["files", "symbols"] {
        for module in ["other-lib", "router-lib/sub"] {
            fs::write(
                root.join("easement.yml"),
                format!(
                    "{POLICY}reach: {reach}\n{}",
                    ALL.replace("module: router-lib", &format!("module: {module}"))
                ),
            )
            .unwrap();
            assert_eq!(check(&root).outcome(), Outcome::Failed);
        }
        fs::write(
            root.join("easement.yml"),
            format!("{POLICY}reach: {reach}\n{ALL}"),
        )
        .unwrap();
        for source in [
            "export { default as Link } from 'router-lib'; export { app } from '../app/router';\n",
            "export { app } from '../app/router';\nexport { default as Link } from 'router-lib';\n",
            "export { default as Link } from 'router-lib';\nexport type { Register } from 'router-lib';\n",
            "export { secret } from '../server/secret';\n",
        ] {
            fs::write(root.join("src/platform/link.ts"), source).unwrap();
            assert_eq!(check(&root).outcome(), Outcome::Failed);
        }
        // Other augmented packages and subpaths remain internal relationships.
        for module in ["other-lib", "router-lib/sub"] {
            fs::write(root.join("src/app/other.ts"), format!("import '{module}';\ndeclare module '{module}' {{ interface Other {{ value: string }} }}\n")).unwrap();
            fs::write(
                root.join("src/platform/link.ts"),
                format!("export {{ default as Link }} from '{module}';\n"),
            )
            .unwrap();
            assert_eq!(check(&root).outcome(), Outcome::Failed);
        }
        fs::remove_file(root.join("src/app/other.ts")).unwrap();
        fs::write(
            root.join("src/platform/link.ts"),
            "export { default as Link } from 'router-lib';\n",
        )
        .unwrap();
    }
}

#[test]
fn project_wide_acceptance_keeps_privacy_cycles_and_external_rules() {
    let (_tmp, root) = fixture("augmentations");
    let policy = POLICY
        .replace("may_depend_on: []", "may_depend_on: [app]")
        .replace(
            "app: { in: src/app }",
            "app: { in: src/app, public: src/app/public.ts }",
        );
    fs::write(root.join("src/app/public.ts"), "export {};\n").unwrap();
    fs::write(root.join("easement.yml"), format!("{policy}{ALL}")).unwrap();
    assert_eq!(check(&root).outcome(), Outcome::Passed);
    fs::write(
        root.join("src/platform/link.ts"),
        "export { default as Link } from 'router-lib'; export { app } from '../app/router';\n",
    )
    .unwrap();
    assert!(text(&check(&root)).contains("[privacy]"));
    let policy = POLICY
        .replace("may_depend_on: []", "may_depend_on: [app]")
        .replace(
            "may_depend_on: [server]",
            "may_depend_on: [server, platform]",
        );
    let original = fs::read_to_string(root.join("src/app/router.ts")).unwrap();
    fs::write(
        root.join("src/app/router.ts"),
        format!(
            "import {{ Link }} from '../platform/link';\n{original}export const linked = Link;\n"
        ),
    )
    .unwrap();
    fs::write(
        root.join("easement.yml"),
        format!("{policy}{ALL}no_cycles: true\n"),
    )
    .unwrap();
    assert!(text(&check(&root)).contains("[cycle]"));
    fs::write(
        root.join("src/platform/link.ts"),
        "export { default as Link } from 'router-lib';\n",
    )
    .unwrap();
    let policy = POLICY.replace(
        "may_depend_on: []",
        "may_depend_on: [], forbid_external: [router-lib]",
    );
    fs::write(root.join("easement.yml"), format!("{policy}{ALL}")).unwrap();
    assert!(text(&check(&root)).contains("[external]"));
}
