//! An alias that is its own definition (#112): `gw` may depend on `chunk` and
//! `chunk` on `blob`, but `gw` not on `blob`. A use of an alias of `chunk` that stands
//! for a name of `blob` is a dependency on `blob`, and the alias's definition is
//! evidence of it.

use std::fs;
use std::path::PathBuf;

use easement::CacheUse;
use easement::commands::{self, CheckOptions, Outcome};
use tempfile::TempDir;

use crate::common::{project, write_all};
use crate::diff::diff_main;
use crate::reexports::{change, found, rust, three};
use crate::run::{
    cache, cache_in, check, check_with, config, json, no_scip_ruby_here, pinned, text,
};

/// The violations `gw -> blob` of `report`, by file and line.
fn gw_to_blob(report: &serde_json::Value) -> Vec<(String, u64)> {
    found(report)
        .into_iter()
        .filter(|(_, _, pair)| pair == "gw -> blob")
        .map(|(file, line, _)| (file, line))
        .collect()
}

/// The violation of `report` at `file`.
fn at<'a>(report: &'a serde_json::Value, file: &str) -> &'a serde_json::Value {
    report["violations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["file"] == file)
        .unwrap_or_else(|| panic!("no violation at {file}: {report}"))
}

/// A Go module of `blob`, `chunk` and `gw` under `internal/`, with `chunk` as given
/// and `gw` of the files given.
fn go(chunk: &str, gw: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let mut files = vec![
        ("go.mod", "module example.com/app\n\ngo 1.25\n"),
        (
            "internal/blob/blob.go",
            "package blob\n\ntype Block struct{ Len int }\n",
        ),
        ("internal/chunk/chunk.go", chunk),
    ];
    files.extend_from_slice(gw);
    three("internal/", &files)
}

/// What `chunk` defines in Go: aliases from line 9 on.
const GO_CHUNK: &str = "package chunk\n\nimport (\n\t\"context\"\n\n\t\"example.com/app/internal/blob\"\n)\n\ntype Block = blob.Block\n\ntype Two = Block\n\ntype Blocks = []blob.Block\n\ntype Def blob.Block\n\ntype Ctx = context.Context\n\ntype Own struct{}\n\ntype Mine = Own\n";

/// A file of `gw` that writes `chunk.<name>` on line 5.
fn go_use(name: &str) -> String {
    format!(
        "package gw\n\nimport \"example.com/app/internal/chunk\"\n\nfunc Use(_ chunk.{name}) {{}}\n"
    )
}

#[test]
fn a_go_alias_is_a_dependency_on_what_it_stands_for_through_chains_and_slices() {
    let uses: Vec<(String, String)> = ["Block", "Two", "Blocks", "Def", "Ctx", "Mine"]
        .iter()
        .map(|name| (format!("internal/gw/{name}.go"), go_use(name)))
        .collect();
    let gw: Vec<(&str, &str)> = uses
        .iter()
        .map(|(file, text)| (file.as_str(), text.as_str()))
        .collect();
    let (_tmp, root) = go(GO_CHUNK, &gw);
    let checked = check(&root);
    let report = json(&checked);
    assert_eq!(checked.outcome(), Outcome::Failed, "{report}");
    // A defined type is a type of its own, `context` is external and `Own` is `chunk`'s.
    assert_eq!(
        found(&report),
        [
            (
                "internal/gw/Block.go".to_string(),
                5,
                "gw -> blob".to_string()
            ),
            (
                "internal/gw/Blocks.go".to_string(),
                5,
                "gw -> blob".to_string()
            ),
            (
                "internal/gw/Two.go".to_string(),
                5,
                "gw -> blob".to_string()
            ),
        ],
        "{report}"
    );
    let block = at(&report, "internal/gw/Block.go");
    assert_eq!(block["reexported_by"], serde_json::json!(["chunk"]));
    assert_eq!(
        block["evidence"],
        serde_json::json!([
            { "file": "internal/gw/Block.go", "line": 5 },
            { "file": "internal/chunk/chunk.go", "line": 9 },
        ]),
        "{block}"
    );
    let two = at(&report, "internal/gw/Two.go");
    assert!(
        two["evidence"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!({ "file": "internal/chunk/chunk.go", "line": 11 })),
        "{two}"
    );
    assert!(
        text(&checked).contains(
            "internal/gw/Block.go:5: gw -> blob is not allowed (written as internal/blob/blob.go, re-exported by chunk)"
        ),
        "{}",
        text(&checked)
    );
}

/// The files of `gw` whose violations `gw -> blob` stand on `line`, `files` sorted.
fn violations_at(files: &[&str], line: u64) -> Vec<(String, u64)> {
    let mut expected: Vec<(String, u64)> = files.iter().map(|f| (f.to_string(), line)).collect();
    expected.sort();
    expected
}

#[test]
fn a_rust_type_alias_is_a_dependency_on_what_it_stands_for_through_chains_and_generics() {
    let (_tmp, root) = rust(
        "pub type Block = blob::Block;\n\npub type Two = Block;\n\npub type Blocks = Vec<blob::Block>;\n",
        "pub mod a;\npub mod b;\npub mod c;\n",
    );
    write_all(
        &root,
        &[
            ("gw/src/a.rs", "pub fn a(_: chunk::Block) {}\n"),
            ("gw/src/b.rs", "pub fn b(_: chunk::Two) {}\n"),
            ("gw/src/c.rs", "pub fn c(_: chunk::Blocks) {}\n"),
        ],
    );
    let report = json(&check(&root));
    assert_eq!(
        gw_to_blob(&report),
        violations_at(&["gw/src/a.rs", "gw/src/b.rs", "gw/src/c.rs"], 1),
        "{report}"
    );
    assert_eq!(
        at(&report, "gw/src/b.rs")["evidence"],
        serde_json::json!([
            { "file": "gw/src/b.rs", "line": 1 },
            { "file": "chunk/src/lib.rs", "line": 1 },
            { "file": "chunk/src/lib.rs", "line": 3 },
        ])
    );
}

#[test]
fn a_typescript_type_alias_is_a_dependency_on_what_it_stands_for_and_a_const_is_not() {
    let (_tmp, root) = three(
        "src/",
        &[
            ("src/blob/index.ts", "export class Block {\n  len = 1;\n}\n"),
            (
                "src/chunk/index.ts",
                "import * as blob from \"../blob\";\n\nexport type Block = blob.Block;\n\nexport type Two = Block;\n\nexport const Made = blob.Block;\n",
            ),
            (
                "src/gw/block.ts",
                "import type { Block } from \"../chunk\";\n\nexport const block = (b: Block) => b;\n",
            ),
            (
                "src/gw/two.ts",
                "import type { Two } from \"../chunk\";\n\nexport const two = (b: Two) => b;\n",
            ),
            (
                "src/gw/made.ts",
                "import { Made } from \"../chunk\";\n\nexport const made = () => new Made();\n",
            ),
        ],
    );
    let report = json(&check(&root));
    // A variable that holds another name is a value: `Made` hides `blob`.
    assert_eq!(
        gw_to_blob(&report),
        violations_at(&["src/gw/block.ts", "src/gw/two.ts"], 1),
        "{report}"
    );
}

#[test]
fn a_ruby_constant_alias_is_a_dependency_on_what_it_stands_for() {
    if no_scip_ruby_here() {
        return;
    }
    let (_tmp, root) = three(
        "lib/",
        &[
            ("Gemfile", "source \"https://rubygems.org\"\n"),
            (
                "lib/blob/block.rb",
                "module Blob\n  class Block\n    def len = 1\n  end\nend\n",
            ),
            (
                "lib/chunk/chunk.rb",
                "module Chunk\n  Block = Blob::Block\n  Two = Block\n  Made = Blob::Block.new\nend\n",
            ),
            (
                "lib/gw/block.rb",
                "module Gw\n  def self.block = Chunk::Block\nend\n",
            ),
            (
                "lib/gw/two.rb",
                "module Gw\n  def self.two = Chunk::Two\nend\n",
            ),
            (
                "lib/gw/made.rb",
                "module Gw\n  def self.made = Chunk::Made\nend\n",
            ),
        ],
    );
    let report = json(&check(&root));
    // `Made` holds a value, which the index relates to nothing.
    assert_eq!(
        gw_to_blob(&report),
        violations_at(&["lib/gw/block.rb", "lib/gw/two.rb"], 2),
        "{report}"
    );
}

/// The index marks no Python alias: an assignment, annotated or not, is a value.
#[test]
fn a_python_assignment_is_no_alias() {
    let (_tmp, root) = three(
        "app/",
        &[
            ("app/__init__.py", ""),
            ("app/blob/__init__.py", "class Block:\n    len = 1\n"),
            (
                "app/chunk/__init__.py",
                "from typing import TypeAlias\n\nfrom app import blob\n\nBlock = blob.Block\nTyped: TypeAlias = blob.Block\n",
            ),
            (
                "app/gw/__init__.py",
                "from app.chunk import Block, Typed\n\n\ndef named(b: Block, t: Typed):\n    return b\n",
            ),
        ],
    );
    let report = json(&check(&root));
    assert_eq!(gw_to_blob(&report), Vec::<(String, u64)>::new(), "{report}");
}

/// `chunk` turns its own `Block` into an alias of `blob`'s, and nothing of `gw` changes.
#[test]
fn an_alias_changed_alone_is_judged_in_a_diff_run() {
    let chunk = |block: &str| {
        format!(
            "package chunk\n\nimport \"example.com/app/internal/blob\"\n\nvar Made = blob.Block{{}}\n\n{block}\n"
        )
    };
    let gw = go_use("Block");
    let (_tmp, root) = go(
        &chunk("type Block struct{}"),
        &[("internal/gw/gw.go", gw.as_str())],
    );
    change(
        &root,
        "internal/chunk/chunk.go",
        &chunk("type Block = blob.Block"),
    );
    let checked = check_with(&root, diff_main);
    let report = json(&checked);
    assert_eq!(checked.outcome(), Outcome::Failed, "{report}");
    assert_eq!(
        found(&report),
        [("internal/gw/gw.go".to_string(), 5, "gw -> blob".to_string())],
        "{report}"
    );
    assert!(
        at(&report, "internal/gw/gw.go")["evidence"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!({ "file": "internal/chunk/chunk.go", "line": 7 })),
        "{report}"
    );
}

/// A ban on `blob.Block` fires on a use of the alias, and one on the alias still does.
#[test]
fn forbid_symbol_sees_a_use_of_an_alias_as_a_use_of_what_it_stands_for() {
    let gw = go_use("Block");
    let (_tmp, root) = go(GO_CHUNK, &[("internal/gw/gw.go", gw.as_str())]);
    for banned in ["blob.Block", "chunk.Block"] {
        fs::write(
            root.join("easement.yml"),
            format!(
                "version: 1\ncomponents:\n  blob: {{ in: internal/blob }}\n  chunk: {{ in: internal/chunk }}\n  gw: {{ in: internal/gw }}\nrules:\n  gw: {{ may_depend_on: [chunk, blob], forbid_symbol: [{banned}] }}\n  chunk: {{ may_depend_on: [blob] }}\n"
            ),
        )
        .unwrap();
        let report = json(&check(&root));
        let fired: Vec<(&str, u64)> = report["violations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["rule"] == "symbol")
            .map(|v| (v["file"].as_str().unwrap(), v["line"].as_u64().unwrap()))
            .collect();
        assert_eq!(fired, [("internal/gw/gw.go", 5)], "{banned}: {report}");
    }
}

/// A baseline entry written before aliases were followed still legalizes the violation.
#[test]
fn a_baselined_dependency_through_an_alias_stays_legalized() {
    let gw = go_use("Block");
    let (_tmp, root) = go(GO_CHUNK, &[("internal/gw/gw.go", gw.as_str())]);
    fs::write(
        root.join("easement.baseline.yml"),
        "violations:\n- rule: dependency\n  file: internal/gw/gw.go\n  target: internal/blob/blob.go\n  from: gw\n  to: blob\n",
    )
    .unwrap();
    let checked = check(&root);
    assert_eq!(checked.outcome(), Outcome::Passed, "{}", text(&checked));
    assert!(text(&checked).contains("1 legalized"), "{}", text(&checked));
}

/// `why` shows the definition of each alias as a hop between the use and the target.
#[test]
fn why_shows_each_alias_behind_a_dependency_as_a_hop() {
    let gw = go_use("Two");
    let (_tmp, root) = go(GO_CHUNK, &[("internal/gw/gw.go", gw.as_str())]);
    let why = commands::why(&config(&root), "gw", "blob", &mut cache()).unwrap();
    let data = serde_json::to_value(&why).unwrap();
    let hop = |file: &str, line: u64, from: &str, to: &str| serde_json::json!({ "file": file, "line": line, "from_component": from, "to_component": to });
    assert_eq!(
        data["dependencies"][0]["aliases"],
        serde_json::json!([{
            "hops": [
                hop("internal/gw/gw.go", 5, "gw", "chunk"),
                hop("internal/chunk/chunk.go", 11, "chunk", "chunk"),
                hop("internal/chunk/chunk.go", 9, "chunk", "blob"),
            ],
            "end": "internal/blob/blob.go",
        }]),
        "{data}"
    );
    assert!(
        why.text().contains(
            "  internal/gw/gw.go:5 depends on internal/blob/blob.go [violates: dependency]\n    through an alias: internal/gw/gw.go:5 -> internal/chunk/chunk.go:11 -> internal/chunk/chunk.go:9 -> internal/blob/blob.go\n"
        ),
        "{}",
        why.text()
    );
}

/// What `check` and `why` print of a dependency through a chain of aliases.
#[test]
fn the_output_of_a_dependency_through_aliases() {
    let gw = go_use("Two");
    let (_tmp, root) = go(GO_CHUNK, &[("internal/gw/gw.go", gw.as_str())]);
    // Indexed afresh, so that no other run decides whether the output says it is cached.
    let own = TempDir::new().unwrap();
    let mut fresh = cache_in(own.path(), CacheUse::Off);
    let checked = commands::check(&CheckOptions::new(config(&root)), &mut fresh).unwrap();
    let report = json(&checked);
    let why = commands::why(&config(&root), "gw", "blob", &mut fresh).unwrap();
    pinned(|| {
        insta::assert_json_snapshot!("check_alias_json", report["violations"]);
        insta::assert_snapshot!("check_alias_text", text(&checked));
        insta::assert_snapshot!("why_alias_text", why.text());
    });
}

/// `blob` opens only its `api`, and its `core` to `chunk` alone. An alias in `chunk`
/// of a name of `core` does not open `core` to `gw`; an alias in `api` does, as a
/// public file's re-export would.
#[test]
fn a_name_reached_through_aliases_alone_is_judged_by_the_surfaces_of_its_component() {
    let (_tmp, root) = project(&[
        ("go.mod", "module example.com/app\n\ngo 1.25\n"),
        (
            "easement.yml",
            "version: 1\ncomponents:\n  blob:\n    in: internal/blob\n    public: internal/blob/api\n    surfaces:\n      core: { in: internal/blob/core, importers: [chunk] }\n  chunk: { in: internal/chunk }\n  gw: { in: internal/gw }\nrules:\n  gw: { may_depend_on: [chunk, blob] }\n  chunk: { may_depend_on: [blob] }\n",
        ),
        (
            "internal/blob/core/core.go",
            "package core\n\ntype Block struct{}\n\ntype Secret struct{}\n",
        ),
        (
            "internal/blob/api/api.go",
            "package api\n\nimport \"example.com/app/internal/blob/core\"\n\ntype Block = core.Block\n",
        ),
        (
            "internal/chunk/chunk.go",
            "package chunk\n\nimport \"example.com/app/internal/blob/core\"\n\ntype Secret = core.Secret\n",
        ),
        (
            "internal/gw/secret.go",
            "package gw\n\nimport \"example.com/app/internal/chunk\"\n\nfunc Secret(_ chunk.Secret) {}\n",
        ),
        (
            "internal/gw/block.go",
            "package gw\n\nimport \"example.com/app/internal/blob/api\"\n\nfunc Block(_ api.Block) {}\n",
        ),
    ]);
    let report = json(&check(&root));
    let privacy: Vec<(&str, u64, &str)> = report["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["file"].as_str().unwrap(),
                v["line"].as_u64().unwrap(),
                v["rule"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        privacy,
        [("internal/gw/secret.go", 5, "privacy")],
        "{report}"
    );
}

/// An alias of an external name adds nothing to the file that uses it: the external
/// module is judged at the import in the alias's own file.
#[test]
fn an_alias_of_an_external_name_counts_against_no_external_rule_of_its_user() {
    let gw = go_use("Ctx");
    let (_tmp, root) = go(GO_CHUNK, &[("internal/gw/gw.go", gw.as_str())]);
    fs::write(
        root.join("easement.yml"),
        "version: 1\ncomponents:\n  blob: { in: internal/blob }\n  chunk: { in: internal/chunk }\n  gw: { in: internal/gw }\nrules:\n  gw: { may_depend_on: [chunk], forbid_external: [context] }\n  chunk: { may_depend_on: [blob] }\n",
    )
    .unwrap();
    let checked = check(&root);
    assert_eq!(checked.outcome(), Outcome::Passed, "{}", text(&checked));
}

/// A public barrel re-exporting a structural type must retain the facade when
/// its aliases are expanded to private implementation types of that component.
#[test]
fn a_public_barrel_keeps_its_alias_dependencies_behind_the_facade() {
    let policy = "version: 1\ncomponents:\n  platform: { in: src/platform, public: src/platform/public.ts }\n  app: { in: src/app }\nrules:\n  app: { may_depend_on: [platform] }\n";
    let (_tmp, root) = project(&[
        ("easement.yml", policy),
        (
            "package.json",
            r#"{"name":"facade-test","version":"1.0.0"}"#,
        ),
        (
            "src/platform/public.ts",
            "export type { Context } from './context';\n",
        ),
        (
            "src/platform/context.ts",
            "import type { Database } from './db';\nexport type Context = { db: Database };\n",
        ),
        (
            "src/platform/db.ts",
            "export type Database = { value: string };\nexport type Secret = string;\n",
        ),
        (
            "src/app/main.ts",
            "import type { Context } from '../platform/public';\nexport const use = (ctx: Context) => ctx.db.value;\n",
        ),
    ]);
    let accepted = check(&root);
    assert_eq!(
        accepted.outcome(),
        easement::commands::Outcome::Passed,
        "{}",
        text(&accepted)
    );
    // Opening a facade for Context never opens ordinary private imports.
    for source in [
        "import type { Context } from '../platform/context';\nexport const use = (ctx: Context) => ctx.db.value;\n",
        "import type { Context } from '../platform/public';\nimport type { Secret } from '../platform/db';\nexport const use = (ctx: Context, secret: Secret) => ctx.db.value + secret;\n",
    ] {
        std::fs::write(root.join("src/app/main.ts"), source).unwrap();
        let rejected = check(&root);
        assert_eq!(
            rejected.outcome(),
            easement::commands::Outcome::Failed,
            "{}",
            text(&rejected)
        );
        assert!(text(&rejected).contains("[privacy]"));
    }
}
