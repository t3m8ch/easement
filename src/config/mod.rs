use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::check::RuleId;
use crate::decision::Grant;
use crate::lang::registry;
use crate::stdlib::STD;
use anyhow::{Context, Result, anyhow, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_norway::Value;

mod schema;
mod validate;

pub use schema::schema_text;
pub use validate::closest;
use validate::unknown_component;

pub const DEFAULT_CONFIG: &str = "easement.yml";
pub const DEFAULT_BASELINE: &str = "easement.baseline.yml";

/// Configuration of the easement architecture quality gate.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = schema::config_schema_extras)]
pub struct Config {
    /// Config format version.
    #[schemars(extend("const" = 1))]
    pub version: u32,
    /// Named parts of the system and the paths they cover.
    #[schemars(extend("minProperties" = 1))]
    pub components: BTreeMap<String, Component>,
    /// Components every other component may depend on.
    #[serde(default)]
    pub common: Vec<String>,
    /// Accept implicit module-string relationships to exactly one augmentation file,
    /// for the listed consumers. Direct imports and explicit symbol uses remain checked.
    #[serde(default)]
    pub accept_augmentations: Vec<AugmentationAcceptance>,
    /// Per component: what it may depend on and use.
    #[serde(default)]
    pub rules: BTreeMap<String, Rule>,
    /// Names that no checked file may use, components and files that belong to none alike,
    /// as `package.Name` or `import/path.Name`, a member after its type (`ent.Client.Query`).
    /// A component's own `forbid_symbol` adds to it. An item may carry its own reason, and
    /// `allow`: the components that may use the name all the same.
    #[serde(default)]
    pub forbid_symbol: Vec<ForbiddenName>,
    /// Globs of paths to skip.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Globs of files (generated code) that stay in the graph as targets, so components can
    /// own them and rules can allow or forbid importing them, but are never checked as the
    /// source of a dependency. `exclude` wins over it.
    #[serde(default)]
    pub target_only: Vec<String>,
    /// Path of the baseline file, default easement.baseline.yml.
    #[serde(default)]
    pub baseline: Option<PathBuf>,
    /// Fail on dependency cycles between components: true for all, or a list of components.
    #[serde(default)]
    pub no_cycles: NoCycles,
    /// What `must_not_reach` follows: `files` (the default) takes an import for everything
    /// its file imports; `symbols` follows the definitions a call references, so calling
    /// one pure method does not reach what another method of its file uses. A file the
    /// index gave no ranges keeps reaching as a whole.
    #[serde(default)]
    pub reach: Reach,
    /// Components from the top layer to the bottom one: a layer may depend on any layer below
    /// it, never on one above. A list in place of a name holds sibling components of one
    /// layer, which do not depend on each other; {name, closed: true} is a layer the ones
    /// above cannot reach past.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Fail on checked files that belong to no component, except those matching `unowned`.
    #[serde(default)]
    pub strict: bool,
    /// Globs of files and directories that may belong to no component in strict mode.
    #[serde(default)]
    pub unowned: Vec<String>,
    /// Source files that no indexer covers and that may stay that way (globs, each with the
    /// reason). Others are listed in the report as not covered, and the result is marked
    /// partial.
    #[serde(default)]
    pub uncovered_ok: Vec<Accepted>,
    /// Settings of an indexer, by the language it reads: `go: { tags: [race] }` indexes Go
    /// files behind `//go:build race` too (files behind other tags are not built, so they
    /// count as not covered). An unknown language or setting fails.
    #[serde(default)]
    pub indexers: crate::lang::registry::IndexerSettings,
    /// `auto` puts the test files of every language (`*_test.go`, `*.test.ts`, `test_*.py`,
    /// `*Test.java`, …) into a component named `tests`, which `rules.tests` can restrict.
    /// Without such a rule tests may import anything.
    #[serde(default)]
    pub tests: Option<TestsMode>,
    /// Files the program starts from (cmd/**, src/main.ts). With it, a checked file of a
    /// component that no chain of imports from them, from a test or from a file of no
    /// component reaches fails as unused. A language whose import graph misses uses is not
    /// judged, and the report says so.
    #[serde(default)]
    pub entry: Vec<String>,
    /// A Markdown file with a Mermaid graph or flowchart of the components (or a .mmd file).
    /// A dependency between components the diagram has no arrow for fails as diagram. Nodes
    /// match components by id or label.
    #[serde(default)]
    pub diagram: Option<PathBuf>,
    /// Every external module a file imports has to be a dependency of the nearest
    /// package.json, go.mod or Cargo.toml above it, else it fails as undeclared. Other
    /// languages are not checked.
    #[serde(default)]
    pub manifests: bool,
    /// Manifests are checked files: each dependency a `Cargo.toml`, `package.json` or `go.mod`
    /// declares is a dependency of the component that owns it, on a package of the project or
    /// on an external module, so the rules apply to it. A dev-dependency is test code, which
    /// `tests: auto` takes. Other manifests are not read yet.
    #[serde(default)]
    pub packages: bool,
    /// Rules whose violations are reported as warnings and do not fail the check. Rules about
    /// the config, such as unmatched, cannot be listed.
    #[serde(default)]
    #[schemars(schema_with = "schema::warn_schema")]
    pub warn: Vec<String>,
    /// Policy examples: proposed dependencies with the decision each must get, judged by
    /// every `check` without any code that imports them. One that gets another decision
    /// fails as `example` at its line, so a later grant cannot silently undo a prohibition.
    /// Paths are relative to the root and need not exist; a source or a target path no
    /// component owns cannot be judged and fails. `extends` joins the lists.
    #[serde(default)]
    pub examples: Vec<Example>,
}

/// An intentional implicit relationship to an external-module augmentation.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AugmentationAcceptance {
    pub consumers: Vec<String>,
    pub module: String,
    /// Exact project-relative path, without globs.
    pub declaration: String,
    pub because: String,
}

impl Config {
    pub fn accepted_augmentation(
        &self,
        consumer: Option<&str>,
        module: Option<&str>,
        declaration: &str,
    ) -> Option<&AugmentationAcceptance> {
        let (consumer, module) = (consumer?, module?);
        self.accept_augmentations.iter().find(|accept| {
            accept.consumers.iter().any(|c| c == consumer)
                && accept.module == module
                && accept.declaration == declaration
        })
    }
}

/// A policy example: a proposed dependency and the decision it must get.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Example {
    /// The file that would hold the import.
    pub from: String,
    /// The path it would import; or `external`.
    #[serde(default)]
    pub to: Option<String>,
    /// The external module it would import; or `to`.
    #[serde(default)]
    pub external: Option<String>,
    /// The decision it must get.
    pub expect: Expect,
    /// The rule that must reject it, when the reason matters; without it any rejection
    /// satisfies `rejected`.
    #[serde(default)]
    pub rule: Option<ExampleRule>,
    /// Why the policy must decide so; shown when it does not.
    #[serde(default)]
    pub because: Option<String>,
}

/// The decision a policy example must get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Expect {
    Allowed,
    Rejected,
}

impl Expect {
    pub fn as_str(self) -> &'static str {
        match self {
            Expect::Allowed => "allowed",
            Expect::Rejected => "rejected",
        }
    }
}

/// The rules that can reject one dependency, and so a policy example.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExampleRule {
    Dependency,
    Privacy,
    Isolation,
    External,
}

impl ExampleRule {
    pub fn id(self) -> RuleId {
        match self {
            ExampleRule::Dependency => RuleId::Dependency,
            ExampleRule::Privacy => RuleId::Privacy,
            ExampleRule::Isolation => RuleId::Isolation,
            ExampleRule::External => RuleId::External,
        }
    }
}

/// The scan-relevant part of the policy ([`Config::scan_policy`]): what the
/// scan of the project reads from it, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanPolicy {
    pub exclude: Vec<String>,
    pub target_only: Vec<String>,
    pub indexers: crate::lang::registry::IndexerSettings,
    pub reach: Reach,
    pub packages: bool,
    pub tests: bool,
    /// Whether any rule needs the uses of names, not only the imports.
    pub uses_symbols: bool,
    /// The names `forbid_symbol` lists, without the qualifier.
    pub watched_symbols: BTreeSet<String>,
}

impl ScanPolicy {
    /// The keys of the policy that make `other` scan differently, in the order
    /// of the fields; empty when one graph serves both.
    pub fn differences(&self, other: &Self) -> Vec<&'static str> {
        // Every field, so that a new one cannot be left out.
        let Self {
            exclude,
            target_only,
            indexers,
            reach,
            packages,
            tests,
            uses_symbols,
            watched_symbols,
        } = self;
        let mut keys = Vec::new();
        let mut differ = |key, differs: bool| {
            if differs {
                keys.push(key);
            }
        };
        differ("exclude", *exclude != other.exclude);
        differ("target_only", *target_only != other.target_only);
        differ("indexers", *indexers != other.indexers);
        differ("reach", *reach != other.reach);
        differ("packages", *packages != other.packages);
        differ("tests", *tests != other.tests);
        // Names are read when `forbid_symbol` or `public_symbols` is set, and
        // `forbid_symbol` says which: with its names alike, `public_symbols` differs.
        let watched = *watched_symbols != other.watched_symbols;
        differ("forbid_symbol", watched);
        differ(
            "public_symbols",
            !watched && *uses_symbols != other.uses_symbols,
        );
        keys
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TestsMode {
    Auto,
}

/// The component that `tests: auto` creates.
pub const TESTS: &str = "tests";

/// A name in a rule's list, optionally with a reason of its own that is shown
/// instead of the reason of the whole rule.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Listed {
    Plain(String),
    Reasoned(Reasoned),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reasoned {
    name: String,
    /// Why this item is listed; shown instead of the reason of the whole rule.
    #[serde(default)]
    because: Option<String>,
}

impl Listed {
    pub fn name(&self) -> &str {
        match self {
            Listed::Plain(name) | Listed::Reasoned(Reasoned { name, .. }) => name,
        }
    }

    pub fn because(&self) -> Option<&str> {
        match self {
            Listed::Plain(_) => None,
            Listed::Reasoned(Reasoned { because, .. }) => because.as_deref(),
        }
    }
}

/// A name in a `forbid_symbol` list: a plain name, or one with a reason of its
/// own and, at the top level, the components that may use it all the same.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ForbiddenName {
    Plain(String),
    Detailed(ForbiddenNameItem),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenNameItem {
    name: String,
    /// Why this name is forbidden; shown instead of the reason of the whole rule.
    #[serde(default)]
    because: Option<String>,
    /// Only in the top-level `forbid_symbol`: the components that may use the name. Every
    /// other use is a violation, in the component that defines the name too.
    #[serde(default)]
    allow: Option<Vec<String>>,
}

impl ForbiddenName {
    pub fn name(&self) -> &str {
        match self {
            ForbiddenName::Plain(name)
            | ForbiddenName::Detailed(ForbiddenNameItem { name, .. }) => name,
        }
    }

    pub fn because(&self) -> Option<&str> {
        match self {
            ForbiddenName::Plain(_) => None,
            ForbiddenName::Detailed(item) => item.because.as_deref(),
        }
    }

    /// The components that may use the name, when the item lists them.
    pub fn allow(&self) -> Option<&[String]> {
        match self {
            ForbiddenName::Plain(_) => None,
            ForbiddenName::Detailed(item) => item.allow.as_deref(),
        }
    }

    /// Whether `component` may use the name although it is forbidden.
    pub fn allows(&self, component: &str) -> bool {
        self.allow()
            .is_some_and(|allowed| allowed.iter().any(|c| c == component))
    }
}

/// An `uncovered_ok` entry: a glob, or `name` with one glob or a list of them
/// that share a reason.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Accepted {
    Plain(String),
    Reasoned(AcceptedGlobs),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptedGlobs {
    /// A glob or a list of globs.
    name: OneOrMany,
    /// Why the files may stay uncovered, reported with the files it accepts.
    #[serde(default)]
    because: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl Accepted {
    pub fn patterns(&self) -> Vec<&str> {
        match self {
            Accepted::Plain(name) => vec![name],
            Accepted::Reasoned(AcceptedGlobs { name, .. }) => match name {
                OneOrMany::One(name) => vec![name],
                OneOrMany::Many(names) => names.iter().map(String::as_str).collect(),
            },
        }
    }

    pub fn because(&self) -> Option<&str> {
        match self {
            Accepted::Plain(_) => None,
            Accepted::Reasoned(AcceptedGlobs { because, .. }) => because.as_deref(),
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NoCycles {
    #[default]
    Off,
    All(bool),
    Among(Vec<String>),
}

impl NoCycles {
    pub fn covers(&self, component: &str) -> bool {
        match self {
            NoCycles::Off => false,
            NoCycles::All(on) => *on,
            NoCycles::Among(list) => list.iter().any(|c| c == component),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// A glob or a list of globs naming a module: a directory or a file. A named placeholder
    /// such as <feature> matches one directory like *, and both ends of an import that name
    /// it must fill it the same way.
    #[serde(rename = "in")]
    pub patterns: Patterns,
    /// When set, other components may import only these paths.
    #[serde(default)]
    pub public: Option<Patterns>,
    /// Why the public surface or the importer list exists; shown with their violations.
    #[serde(default)]
    pub because: Option<String>,
    /// Files matching these patterns do not belong to this component, even though `in`
    /// matches them.
    #[serde(default)]
    pub except: Option<Patterns>,
    /// Which component wins when several match a file: the higher priority, then the longer
    /// pattern. Default 0.
    #[serde(default)]
    pub priority: i32,
    /// The directories matched by `in` (each pattern needs a `*`, no `**`) are instances that
    /// must not import each other.
    #[serde(default)]
    pub isolated: bool,
    /// When set, other components may use only these names from this component's packages.
    /// In a language whose uses of names easement does not read, the report has a `symbols`
    /// evidence gap instead.
    #[serde(default)]
    pub public_symbols: Option<Vec<String>>,
    /// When set, only these components (and the component itself) may import it.
    #[serde(default)]
    pub allow_importers: Option<Vec<String>>,
    /// Named surfaces: paths of the component open only to the components each lists. With
    /// surfaces and no `public`, the public surface is empty.
    #[serde(default)]
    pub surfaces: BTreeMap<String, Surface>,
    /// The component may match no file. Otherwise an `in` pattern that matches no file of the
    /// project fails as `unmatched`. Default false.
    #[serde(default)]
    pub optional: bool,
    /// Who to ask about the component and its rules: a team or a person. Printed with the
    /// violations of the component.
    #[serde(default)]
    pub owner: Option<String>,
    /// Globs of type names that belong in this component (*Repository): a class, struct,
    /// interface or trait so named that another component defines fails as naming. Types are
    /// read from the index; files no index reads are an evidence gap.
    #[serde(default)]
    pub defines: Vec<String>,
    /// With `packages`, the part of the files `in` matches that the component owns, as the
    /// language or manifest names it: `feature/fake` for the Cargo feature `fake`. The file
    /// itself stays with the component that has no `part`.
    #[serde(default)]
    pub part: Option<String>,
}

impl Component {
    /// The paths of the public surface, when the component restricts it:
    /// `public`, or none when it declares only named `surfaces`.
    pub fn public_surface(&self) -> Option<Vec<&str>> {
        match &self.public {
            Some(public) => Some(public.as_vec()),
            None if !self.surfaces.is_empty() => Some(Vec::new()),
            None => None,
        }
    }
}

/// A named surface: paths of a component that only the listed components may import.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    /// A glob or a list of globs inside the component, naming a module as `public` does.
    #[serde(rename = "in")]
    pub patterns: Patterns,
    /// The components that may import these paths; each must be in `allow_importers` when the
    /// component has one.
    #[schemars(length(min = 1))]
    pub importers: Vec<String>,
    /// Why the surface exists; shown with its violations.
    #[serde(default)]
    pub because: Option<String>,
}

/// One layer of `layers`: a component, sibling components that do not
/// depend on each other, or a closed component that the layers above it
/// cannot reach past.
#[derive(Debug, JsonSchema)]
#[schemars(untagged)]
pub enum Layer {
    One(String),
    Siblings(#[schemars(length(min = 2))] Vec<String>),
    Named(NamedLayer),
}

impl<'de> Deserialize<'de> for Layer {
    /// By the shape of the value, so a mistyped key of a named layer
    /// (`close:`) is named in the error, not lost in "no variant matched".
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match Value::deserialize(deserializer)? {
            Value::String(name) => Ok(Layer::One(name)),
            seq @ Value::Sequence(_) => serde_norway::from_value(seq)
                .map(Layer::Siblings)
                .map_err(|e| D::Error::custom(format!("a list of sibling layers: {e}"))),
            map @ Value::Mapping(_) => serde_norway::from_value(map)
                .map(Layer::Named)
                .map_err(|e| D::Error::custom(format!("a layer {{ name, closed }}: {e}"))),
            _ => Err(D::Error::custom(
                "a layer is a component name, a list of siblings or { name, closed }",
            )),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedLayer {
    name: String,
    /// The layers above may not reach past this one.
    #[serde(default)]
    closed: bool,
}

impl Layer {
    pub fn names(&self) -> Vec<&str> {
        match self {
            Layer::One(name) | Layer::Named(NamedLayer { name, .. }) => vec![name],
            Layer::Siblings(names) => names.iter().map(String::as_str).collect(),
        }
    }

    fn closed(&self) -> bool {
        matches!(self, Layer::Named(NamedLayer { closed: true, .. }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Patterns {
    One(String),
    Many(#[schemars(length(min = 1))] Vec<String>),
}

impl Patterns {
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Patterns::One(p) => vec![p.as_str()],
            Patterns::Many(ps) => ps.iter().map(String::as_str).collect(),
        }
    }
}

/// What a chain of `must_not_reach` follows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Reach {
    #[default]
    Files,
    Symbols,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Components this one may depend on. An item may carry its own reason, shown by
    /// `explain`.
    #[serde(default)]
    pub may_depend_on: Vec<Listed>,
    /// Components that are never allowed, even if common or layers would allow them. An item
    /// may carry its own reason.
    #[serde(default)]
    pub must_not_depend_on: Vec<Listed>,
    /// Components this component must not depend on, directly or through any chain of
    /// imports, even when every hop is allowed. An item may carry its own reason.
    #[serde(default)]
    pub must_not_reach: Vec<Listed>,
    /// External modules this component may use; unset means no restriction. A pattern covers
    /// submodules: lodash covers lodash/fp. $std stands for the standard library of the
    /// importing file's language. An item may carry its own reason.
    #[serde(default)]
    pub may_use_external: Option<Vec<Listed>>,
    /// External modules this component must not use. $std stands for the standard library of
    /// the importing file's language. An item may carry its own reason.
    #[serde(default)]
    pub forbid_external: Vec<Listed>,
    /// Names this component must not use, as `package.Name` or `import/path.Name`
    /// (`ent.Client`), a member after its type (`ent.Client.Query`). In a language whose uses
    /// of names easement does not read, the report has a `symbols` evidence gap instead. An
    /// item may carry its own reason.
    #[serde(default)]
    // Read with `allow` so that loading refuses it by name; editors are not offered it.
    #[schemars(with = "Vec<Listed>")]
    pub forbid_symbol: Vec<ForbiddenName>,
    /// Components that every directory of this component must import at least once. An item
    /// may carry its own reason.
    #[serde(default)]
    pub must_depend_on: Vec<Listed>,
    /// External modules that every directory of this component must use; a pattern covers
    /// submodules. An item may carry its own reason.
    #[serde(default)]
    pub must_use_external: Vec<Listed>,
    /// File names or globs that every directory of this component must contain. An item may
    /// carry its own reason.
    #[serde(default)]
    pub requires_file: Vec<Listed>,
    /// File names or globs that no file of this component may match. An item may carry its
    /// own reason.
    #[serde(default)]
    pub forbid_file: Vec<Listed>,
    /// Regular expressions that some file in every directory of this component must match,
    /// for dependencies written as comments (`//go:embed`, `goverter:converter`). An item may
    /// carry its own reason.
    #[serde(default)]
    pub requires_directive: Vec<Listed>,
    /// Most components this one may depend on.
    #[serde(default)]
    pub max_fan_out: Option<usize>,
    /// Highest instability, out / (in + out), from 0 (stable) to 1.
    #[serde(default)]
    pub max_instability: Option<f64>,
    /// Why the rule exists; shown with its violations.
    #[serde(default)]
    pub because: Option<String>,
}

impl Config {
    /// The config of a value merged from the configuration at `path` and the
    /// files it extends (`policy::Policy::load`); `path` names it in errors.
    pub fn from_merged(value: Value, path: &Path) -> Result<Self> {
        // The merged value has no line numbers, so the error names the path to the field.
        let config: Config = serde_path_to_error::deserialize(value)
            .with_context(|| format!("invalid config {}", path.display()))?;
        let config = config.with_tests()?;
        config.validate()?;
        Ok(config)
    }

    /// Fails when `name` is no component, naming the one it is likely a typo of.
    pub fn require_component(&self, name: &str) -> Result<()> {
        if self.components.contains_key(name) {
            return Ok(());
        }
        Err(unknown_component(
            name,
            "",
            self.components.keys().map(String::as_str),
        ))
    }

    /// The scan-relevant part of the policy: everything the scan reads. Two
    /// policies with an equal part can be judged on one graph.
    pub fn scan_policy(&self) -> ScanPolicy {
        ScanPolicy {
            exclude: self.exclude.clone(),
            target_only: self.target_only.clone(),
            indexers: self.indexers.clone(),
            reach: self.reach,
            packages: self.packages,
            tests: self.tests.is_some(),
            uses_symbols: self.uses_symbols(),
            watched_symbols: self.forbidden_symbol_names(),
        }
    }

    /// Whether any rule needs the uses of names, not only the imports.
    fn uses_symbols(&self) -> bool {
        self.components.values().any(|c| c.public_symbols.is_some())
            || !self.forbid_symbol.is_empty()
            || self.rules.values().any(|r| !r.forbid_symbol.is_empty())
    }

    /// The names that `forbid_symbol` lists, without the qualifier: `locale` of `I18n.locale`.
    fn forbidden_symbol_names(&self) -> BTreeSet<String> {
        self.forbid_symbol
            .iter()
            .chain(
                self.rules
                    .values()
                    .flat_map(|rule| rule.forbid_symbol.iter()),
            )
            .filter_map(|listed| listed.name().rsplit_once('.'))
            .map(|(_, name)| name.to_string())
            .collect()
    }

    /// Adds the `tests` component when `tests: auto` asks for it.
    fn with_tests(mut self) -> Result<Self> {
        if self.tests.is_none() {
            return Ok(self);
        }
        if self.components.contains_key(TESTS) {
            bail!("`tests: auto` creates a component named `{TESTS}`, which is already declared");
        }
        self.components.insert(
            TESTS.to_string(),
            Component {
                patterns: Patterns::Many(
                    registry::test_patterns()
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                ),
                public: None,
                because: None,
                except: Some(Patterns::Many(
                    registry::not_test_patterns()
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                )),
                priority: 1_000_000,
                isolated: false,
                public_symbols: None,
                allow_importers: None,
                surfaces: BTreeMap::new(),
                // A project may have no tests yet.
                optional: true,
                owner: None,
                defines: Vec::new(),
                part: None,
            },
        );
        Ok(self)
    }

    /// The owner of each component that names one.
    pub fn owners(&self) -> std::collections::BTreeMap<String, String> {
        self.components
            .iter()
            .filter_map(|(name, c)| Some((name.clone(), c.owner.clone()?)))
            .collect()
    }

    /// Whether violations of `rule` are warnings rather than failures.
    /// The baseline file, relative to the root.
    pub fn baseline_file(&self) -> PathBuf {
        self.baseline
            .clone()
            .unwrap_or_else(|| DEFAULT_BASELINE.into())
    }

    pub fn warns(&self, rule: RuleId) -> bool {
        self.warn.iter().any(|w| w == rule.as_str())
    }

    pub fn reason(&self, from: &str) -> Option<&str> {
        self.rules.get(from).and_then(|r| r.because.as_deref())
    }

    pub fn forbids(&self, from: &str, to: &str) -> bool {
        self.rules.get(from).is_some_and(|r| {
            r.must_not_depend_on
                .iter()
                .chain(&r.must_not_reach)
                .any(|c| c.name() == to)
        })
    }

    /// Whether `must_not_reach` of `from` names `to`: not even a chain of
    /// imports through other components may reach it.
    pub fn forbids_reach(&self, from: &str, to: &str) -> bool {
        self.rules
            .get(from)
            .is_some_and(|r| r.must_not_reach.iter().any(|c| c.name() == to))
    }

    /// The reason given with `to` in the `must_not_depend_on` or
    /// `must_not_reach` of `from`, if any.
    pub fn forbid_reason(&self, from: &str, to: &str) -> Option<&str> {
        let rule = self.rules.get(from)?;
        rule.must_not_depend_on
            .iter()
            .chain(&rule.must_not_reach)
            .find(|c| c.name() == to)?
            .because()
    }

    /// The reason given with `to` in the `may_depend_on` of `from`, if any.
    pub fn allow_reason(&self, from: &str, to: &str) -> Option<&str> {
        self.rules
            .get(from)?
            .may_depend_on
            .iter()
            .find(|c| c.name() == to)?
            .because()
    }

    /// What lets `from` depend on `to`, when something does and nothing
    /// forbids it.
    pub fn grant(&self, from: &str, to: &str) -> Option<Grant> {
        if from == to {
            return Some(Grant::SameComponent);
        }
        if self.forbids(from, to) {
            return None;
        }
        if self.tests.is_some() && from == TESTS && !self.rules.contains_key(TESTS) {
            Some(Grant::Tests)
        } else if self
            .rules
            .get(from)
            .is_some_and(|r| r.may_depend_on.iter().any(|c| c.name() == to))
        {
            Some(Grant::MayDependOn)
        } else if self.common.iter().any(|c| c == to) {
            Some(Grant::Common)
        } else if self.above(from, to) {
            Some(Grant::Layers)
        } else {
            None
        }
    }

    /// Why `layers` keep `from` off `to` although both are layers, if they do:
    /// they are siblings, or a closed layer lies between them.
    pub fn layer_block(&self, from: &str, to: &str) -> Option<String> {
        let at = |name: &str| self.layers.iter().position(|l| l.names().contains(&name));
        let (f, t) = (at(from)?, at(to)?);
        if f == t && from != to {
            return Some("they are siblings in one layer".to_string());
        }
        let closed = self.layers.get(f + 1..t)?.iter().find(|l| l.closed())?;
        Some(format!(
            "the closed layer {} is between them",
            closed.names().join(", ")
        ))
    }

    /// Whether `from` is a higher layer than `to`, with no closed layer between.
    fn above(&self, from: &str, to: &str) -> bool {
        let at = |name: &str| self.layers.iter().position(|l| l.names().contains(&name));
        // A closed layer between them stops the layers above from reaching past it.
        matches!((at(from), at(to)), (Some(f), Some(t))
            if f < t && !self.layers[f + 1..t].iter().any(Layer::closed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_relevant_parts_of_two_policies_name_the_keys_they_differ_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("easement.yml");
        let scan = |text: &str| {
            std::fs::write(&path, text).unwrap();
            load(&path).unwrap().scan_policy()
        };
        let base = scan("version: 1\ncomponents:\n  a: { in: a }\nexclude: [gen]\n");
        let same = scan(
            "version: 1\ncomponents:\n  a: { in: a, owner: x }\nexclude: [gen]\nstrict: true\n",
        );
        assert!(base.differences(&same).is_empty());
        let head = scan(
            "version: 1\ncomponents:\n  a: { in: a }\nexclude: [gen, vendor]\ntests: auto\nforbid_symbol: [\"d.T\"]\n",
        );
        assert_eq!(
            base.differences(&head),
            ["exclude", "tests", "forbid_symbol"]
        );
    }

    #[test]
    fn a_top_level_forbid_symbol_counts_as_a_use_of_names_and_must_name_a_package() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("easement.yml");
        std::fs::write(
            &path,
            "version: 1
components:
  a: { in: a }
forbid_symbol: [\"d.T\"]
",
        )
        .unwrap();
        let scan = load(&path).unwrap().scan_policy();
        assert!(scan.uses_symbols);
        assert!(scan.watched_symbols.contains("T"));
        std::fs::write(
            &path,
            "version: 1
components:
  a: { in: a }
forbid_symbol: [T]
",
        )
        .unwrap();
        let error = format!("{:#}", load(&path).unwrap_err());
        assert!(error.contains("must look like `package.Name`"), "{error}");
    }

    #[test]
    fn merge_keys_share_a_part_of_several_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("easement.yml");
        std::fs::write(
            &path,
            "version: 1
components:
  a: { in: a }
  b: { in: b }
  c: { in: c }
x-ban: &ban
  forbid_symbol:
    - { name: \"d.T\", because: no raw clients }
rules:
  a: { <<: *ban, may_depend_on: [b] }
  b: { <<: *ban }
",
        )
        .unwrap();
        let c = load(&path).unwrap();
        assert_eq!(c.rules["a"].forbid_symbol.len(), 1);
        assert_eq!(c.rules["b"].forbid_symbol[0].name(), "d.T");
        assert_eq!(c.rules["a"].may_depend_on.len(), 1);
        assert!(c.rules["b"].may_depend_on.is_empty());

        // Only keys that start with `x-` are free; a typo is still an error.
        std::fs::write(&path, "version: 1\ncomponents:\n  a: { in: a }\nban: {}\n").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn a_config_error_names_the_path_to_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("easement.yml");
        std::fs::write(
            &path,
            "version: 1\ncomponents:\n  a: { in: a }\n  b: { in: b, publik: [x] }\n",
        )
        .unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains("components.b.publik: unknown field `publik`"),
            "{err}"
        );
    }

    fn load(path: &Path) -> Result<Config> {
        Ok(crate::policy::Policy::load(path, None)?.config)
    }

    fn parse(yaml: &str) -> Result<Config> {
        let config: Config = serde_norway::from_str(yaml)?;
        let config = config.with_tests()?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn allows_self_common_and_declared() {
        let c = parse(
            "version: 1
components:
  a: { in: a }
  b: { in: b }
  c: { in: c }
  m: { in: m }
common: [m]
rules:
  a: { may_depend_on: [b] }
",
        )
        .unwrap();
        assert!(c.grant("a", "a").is_some());
        assert!(c.grant("a", "b").is_some());
        assert!(c.grant("c", "m").is_some());
        assert!(!c.grant("a", "c").is_some());
        assert!(!c.grant("b", "a").is_some());
    }

    #[test]
    fn sibling_layers_are_independent_and_a_closed_layer_cannot_be_skipped() {
        let c = parse(
            "version: 1
components:
  web: { in: web }
  api: { in: api }
  jobs: { in: jobs }
  service: { in: service }
  repo: { in: repo }
layers:
  - web
  - [api, jobs]
  - { name: service, closed: true }
  - repo
",
        )
        .unwrap();
        assert!(c.grant("web", "api").is_some());
        assert!(!c.grant("api", "jobs").is_some());
        assert!(!c.grant("jobs", "api").is_some());
        assert!(c.grant("api", "service").is_some());
        assert!(c.grant("service", "repo").is_some());
        // Nothing above `service` reaches past it.
        assert!(!c.grant("api", "repo").is_some());
        assert!(!c.grant("web", "repo").is_some());
        assert!(!c.grant("repo", "web").is_some());
        assert_eq!(
            c.layer_block("api", "repo").as_deref(),
            Some("the closed layer service is between them")
        );
        assert_eq!(
            c.layer_block("api", "jobs").as_deref(),
            Some("they are siblings in one layer")
        );
        assert_eq!(c.layer_block("service", "repo"), None);
        let typo = "version: 1\ncomponents:\n  a: { in: a }\nlayers: [{ name: a, close: true }]\n";
        assert!(format!("{:#}", parse(typo).unwrap_err()).contains("unknown field `close`"));
        let lone = "version: 1\ncomponents:\n  a: { in: a }\nlayers: [[a]]\n";
        assert!(format!("{:#}", parse(lone).unwrap_err()).contains("fewer than two siblings"));
        let twice =
            "version: 1\ncomponents:\n  a: { in: a }\n  b: { in: b }\nlayers: [a, [a, b]]\n";
        assert!(format!("{:#}", parse(twice).unwrap_err()).contains("twice"));
    }

    #[test]
    fn layers_allow_downwards_and_forbidden_wins() {
        let c = parse(
            "version: 1
components:
  top: { in: top }
  mid: { in: mid }
  low: { in: low }
  m: { in: m }
common: [m]
layers: [top, mid, low]
rules:
  top: { must_not_depend_on: [low, m] }
",
        )
        .unwrap();
        assert!(c.grant("top", "mid").is_some());
        assert!(c.grant("mid", "low").is_some());
        assert!(!c.grant("mid", "top").is_some());
        assert!(!c.grant("low", "mid").is_some());
        assert!(!c.grant("top", "low").is_some());
        assert!(!c.grant("top", "m").is_some());
        assert!(c.grant("mid", "m").is_some());
    }

    #[test]
    fn listed_names_carry_their_own_reason() {
        let c = parse(
            "version: 1
components:
  a: { in: a }
  b: { in: b }
  c: { in: c }
rules:
  a:
    must_not_depend_on: [{ name: b, because: why b }, c]
    because: why a
",
        )
        .unwrap();
        assert!(c.forbids("a", "b") && c.forbids("a", "c"));
        assert_eq!(c.forbid_reason("a", "b"), Some("why b"));
        assert_eq!(c.forbid_reason("a", "c"), None);
        assert!(
            parse(
                "version: 1
components:
  a: { in: a }
rules:
  a: { forbid_external: [{ name: x, because: y, oops: z }] }
"
            )
            .is_err()
        );
    }

    /// An example names one target, and only a rule that can reject that kind
    /// of target, only when it expects a rejection.
    #[test]
    fn tests_auto_adds_a_component_that_is_open_unless_ruled() {
        let open = parse(
            "version: 1
components:
  a: { in: a }
tests: auto
",
        )
        .unwrap();
        assert!(open.components.contains_key("tests"));
        assert!(open.grant("tests", "a").is_some());
        assert!(!open.grant("a", "tests").is_some());
        let ruled = parse(
            "version: 1
components:
  a: { in: a }
  helper: { in: testhelper }
tests: auto
rules:
  tests: { may_depend_on: [helper] }
",
        )
        .unwrap();
        assert!(ruled.grant("tests", "helper").is_some());
        assert!(!ruled.grant("tests", "a").is_some());
        let err = parse(
            "version: 1
components:
  tests: { in: t }
tests: auto
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("already declared"));
    }

    #[test]
    fn may_depend_on_carries_reasons() {
        let c = parse(
            "version: 1
components:
  a: { in: a }
  b: { in: b }
rules:
  a: { may_depend_on: [{ name: b, because: why b }] }
",
        )
        .unwrap();
        assert!(c.grant("a", "b").is_some());
        assert_eq!(c.allow_reason("a", "b"), Some("why b"));
    }
}
