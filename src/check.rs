use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::components::ComponentMap;
use crate::config::{Config, Listed};
use crate::decision::{self, Clause, Decision, Refusal};
use crate::lang::Target;
use crate::lang::{Dep, Mention, Through};
use crate::policy::Provenance;
use crate::relpath;
use crate::relpath::slash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleId {
    /// An import between components that `may_depend_on` does not allow.
    Dependency,
    /// A dependency on a path outside the target component's public surface.
    Privacy,
    /// A dependency cycle between components.
    Cycle,
    /// A component uses an external module its rule does not allow.
    External,
    /// A component exceeds a coupling limit.
    Metric,
    /// A file that belongs to no component, in strict mode.
    Orphan,
    /// An import between instances of an isolated component.
    Isolation,
    /// A directory of a component lacks an import or a file the rules require.
    Required,
    /// A file a component's rules forbid.
    File,
    /// A use of a name the rules forbid or the target does not make public.
    Symbol,
    /// An inline suppression that matches no violation.
    UnusedSuppression,
    /// An inline suppression without a reason.
    SuppressionReason,
    /// An `in` pattern of a component that matches no file of the project.
    Unmatched,
    /// An import of a file that `exclude` hides.
    Excluded,
    /// A chain of imports from a component to one its `must_not_reach` names.
    Reach,
    /// A file of a component that no chain of imports from an `entry` reaches.
    Unused,
    /// A type whose name a component's `defines` claims, defined elsewhere.
    Naming,
    /// A dependency between components that the `diagram` has no arrow for.
    Diagram,
    /// An external module the nearest manifest does not declare.
    Undeclared,
    /// A policy example that the policy decides otherwise than it expects.
    Example,
}

impl RuleId {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleId::Dependency => "dependency",
            RuleId::Privacy => "privacy",
            RuleId::Cycle => "cycle",
            RuleId::External => "external",
            RuleId::Metric => "metric",
            RuleId::Orphan => "orphan",
            RuleId::Isolation => "isolation",
            RuleId::Required => "required",
            RuleId::File => "file",
            RuleId::Symbol => "symbol",
            RuleId::UnusedSuppression => "unused-suppression",
            RuleId::SuppressionReason => "suppression-reason",
            RuleId::Unmatched => "unmatched",
            RuleId::Excluded => "excluded",
            RuleId::Reach => "reach",
            RuleId::Unused => "unused",
            RuleId::Naming => "naming",
            RuleId::Diagram => "diagram",
            RuleId::Undeclared => "undeclared",
            RuleId::Example => "example",
        }
    }

    /// Rules about the config rather than the code: no import or changed line
    /// stands behind them, so `--diff` and `--scope` keep them, and the
    /// baseline never accepts them.
    pub fn is_config_rule(self) -> bool {
        matches!(self, RuleId::Unmatched | RuleId::Example)
    }

    /// Every rule, in the order of the declaration.
    pub const ALL: [RuleId; 20] = [
        RuleId::Dependency,
        RuleId::Privacy,
        RuleId::Cycle,
        RuleId::External,
        RuleId::Metric,
        RuleId::Orphan,
        RuleId::Isolation,
        RuleId::Required,
        RuleId::File,
        RuleId::Symbol,
        RuleId::UnusedSuppression,
        RuleId::SuppressionReason,
        RuleId::Unmatched,
        RuleId::Excluded,
        RuleId::Reach,
        RuleId::Unused,
        RuleId::Naming,
        RuleId::Diagram,
        RuleId::Undeclared,
        RuleId::Example,
    ];

    /// Rules that an inline comment can suppress.
    pub const SUPPRESSIBLE: [RuleId; 8] = [
        RuleId::Dependency,
        RuleId::Privacy,
        RuleId::External,
        RuleId::Isolation,
        RuleId::Symbol,
        RuleId::Excluded,
        RuleId::Naming,
        RuleId::Undeclared,
    ];

    pub fn suppressible(s: &str) -> Option<Self> {
        Self::SUPPRESSIBLE.into_iter().find(|r| r.as_str() == s)
    }
}

fn default_rule() -> RuleId {
    RuleId::Dependency
}

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: RuleId,
    pub file: String,
    pub line: usize,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub because: Option<String>,
    pub from: String,
    pub to: String,
    pub target: String,
    pub specifier: String,
    /// For a name written through a module of another component that re-exports
    /// it, or through an alias another component defines: those components, whose
    /// lines that refer to the target are in `evidence`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reexported_by: Vec<String>,
    /// Every import behind the violation; diff mode keeps it if any of them changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Place>,
    /// For a reach or a cycle: the shortest chain of imports that shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<Chain>,
    /// Where the policy writes the clause the violation breaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

/// A place in the code, as `{file, line}`, and the module an import writes
/// there as `specifier` when that is what the place is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Place {
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specifier: Option<String>,
}

impl Place {
    /// The place at `line` of `file`, about nothing more.
    pub fn new(file: impl Into<String>, line: usize) -> Self {
        Self {
            file: file.into(),
            line,
            specifier: None,
        }
    }
}

/// `file:line`, then `` (`specifier`)`` when the place has one.
impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)?;
        if let Some(specifier) = &self.specifier {
            write!(f, " (`{specifier}`)")?;
        }
        Ok(())
    }
}

/// The first `shown` of `items`, each as `name` gives it, joined by `separator`,
/// then what `more` says of how many it leaves out, if any: every list that a
/// report names only in part is cut so.
pub fn name_first<T>(
    items: &[T],
    shown: usize,
    separator: &str,
    name: impl Fn(&T) -> String,
    more: impl Fn(usize) -> String,
) -> String {
    let mut named = items
        .iter()
        .take(shown)
        .map(name)
        .collect::<Vec<_>>()
        .join(separator);
    if items.len() > shown {
        named.push_str(&more(items.len() - shown));
    }
    named
}

/// One dependency of a chain: where it is written, and the components it leaves
/// and enters; none for a file of no component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Hop {
    pub file: String,
    pub line: usize,
    pub from_component: Option<String>,
    pub to_component: Option<String>,
}

/// A chain of dependencies that shows a violation: one witness,
/// the shortest, not every import involved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chain {
    /// Always `shortest`: the chain is one witness among those that may exist.
    pub witness: &'static str,
    pub hops: Vec<Hop>,
}

impl Chain {
    /// The chain of a [`shortest_chain`]: each hop enters the component of the
    /// next hop's file, the last the component of the file it ends in.
    pub fn of<'a>(hops: &[Place], last: &str, of: impl Fn(&str) -> Option<&'a str>) -> Self {
        let next = hops.iter().skip(1).map(|hop| hop.file.as_str());
        Self::shortest(
            hops.iter()
                .zip(next.chain(std::iter::once(last)))
                .map(|(Place { file, line, .. }, to)| Hop {
                    file: file.clone(),
                    line: *line,
                    from_component: of(file).map(str::to_string),
                    to_component: of(to).map(str::to_string),
                })
                .collect(),
        )
    }

    pub(crate) fn shortest(hops: Vec<Hop>) -> Self {
        Self {
            witness: "shortest",
            hops,
        }
    }
}

/// Identity of a violation for the baseline: line numbers are left out so that
/// unrelated edits do not invalidate it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Key {
    #[serde(default = "default_rule")]
    pub rule: RuleId,
    pub file: String,
    pub target: String,
    pub from: String,
    pub to: String,
}

impl Violation {
    /// The components `from` names: the one a dependency leaves, or each member of
    /// a cycle, which `from` lists as `a,b`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.from.split(',')
    }

    /// A violation of `rule` with every other field empty, for a literal to fill in:
    /// `Violation { file, line, message, ..Violation::of(rule) }`.
    pub fn of(rule: RuleId) -> Self {
        Self {
            rule,
            file: String::new(),
            line: 0,
            message: String::new(),
            because: None,
            from: String::new(),
            to: String::new(),
            target: String::new(),
            specifier: String::new(),
            reexported_by: Vec::new(),
            evidence: Vec::new(),
            chain: None,
            provenance: None,
        }
    }

    pub fn key(&self) -> Key {
        Key {
            rule: self.rule,
            file: self.file.clone(),
            target: self.target.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
        }
        .canonical()
    }
}

impl Key {
    /// The key without the file a violation happens to be reported at, where
    /// the rule is about something else: a cycle, a limit and a reach are
    /// about components, a requirement about a directory. Another file that sorts
    /// first then does not turn old debt into new.
    pub fn canonical(mut self) -> Self {
        match self.rule {
            RuleId::Cycle | RuleId::Metric | RuleId::Diagram => self.file.clear(),
            // Which file of the reached component the chain ends in changes as
            // freely as where it starts.
            RuleId::Reach => {
                self.file.clear();
                self.target.clear();
            }
            // The directory ends in `/`, so a canonical key stays as it is.
            RuleId::Required if !self.file.ends_with('/') => {
                self.file = self
                    .file
                    .rsplit_once('/')
                    .map_or_else(|| "./".to_string(), |(dir, _)| format!("{dir}/"));
            }
            _ => {}
        }
        self
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Code a rule could not read or judge: why, in one sentence, and the files or
/// the extensions of the files it holds for, where they are known, and the
/// places in them, where those are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unjudged {
    pub reason: String,
    pub files: Vec<String>,
    pub extensions: Vec<String>,
    pub places: Vec<Place>,
}

impl Unjudged {
    pub fn new(reason: String) -> Self {
        Self {
            reason,
            ..Self::default()
        }
    }

    /// The files it holds for, sorted, each once.
    pub fn in_files<S: Into<String>>(mut self, files: impl IntoIterator<Item = S>) -> Self {
        self.files = files.into_iter().map(Into::into).collect();
        self.files.sort();
        self.files.dedup();
        self
    }

    /// The places it holds at, sorted, each once.
    pub fn at(mut self, places: impl IntoIterator<Item = Place>) -> Self {
        self.places = places.into_iter().collect();
        self.places.sort();
        self.places.dedup();
        self
    }

    /// The extensions of the files it holds for, sorted, each once.
    pub fn in_extensions<S: Into<String>>(
        mut self,
        extensions: impl IntoIterator<Item = S>,
    ) -> Self {
        self.extensions = extensions.into_iter().map(Into::into).collect();
        self.extensions.sort();
        self.extensions.dedup();
        self
    }
}

#[derive(Debug, Serialize)]
pub struct AcceptedAugmentation {
    pub file: String,
    pub line: usize,
    pub module: String,
    pub declaration: String,
    pub because: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Stats {
    /// Relationships deliberately omitted from dependency and reach judgments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accepted_augmentations: Vec<AcceptedAugmentation>,
    pub files: usize,
    pub internal_deps: usize,
    pub external_deps: usize,
    pub unmapped_files: usize,
    pub suppressed: usize,
    /// Indexers that ran, and indexes taken from the cache instead.
    #[serde(skip_serializing_if = "is_zero")]
    pub indexed: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub from_cache: usize,
    /// File extensions whose standard library `$std` cannot tell, among the
    /// files of components whose rules name it: their imports were judged by
    /// name only.
    #[serde(skip)]
    pub std_unknown: Vec<String>,
    /// Extensions of checked component files whose types `defines` cannot
    /// see: they are read with tree-sitter, which reports no definitions.
    #[serde(skip)]
    pub naming_unseen: Vec<String>,
    /// Why `manifests` left some imports unchecked.
    #[serde(skip)]
    pub manifests_partial: Vec<Unjudged>,
    /// Test code that `tests: auto` could not take out of the file it sits in.
    #[serde(skip)]
    pub parts_partial: Vec<Unjudged>,
    /// Manifests, or dependencies of one, that `packages` could not judge as declared.
    #[serde(skip)]
    pub packages_partial: Vec<Unjudged>,
    /// Imports a file writes that the indexer did not resolve.
    #[serde(skip)]
    pub imports_partial: Vec<Unjudged>,
    /// Why `forbid_symbol` left some uses of names unchecked.
    #[serde(skip)]
    pub symbols_partial: Vec<Unjudged>,
    /// Why `unused` took some files as used without judging them.
    #[serde(skip)]
    pub unused_partial: Vec<Unjudged>,
    /// Source files of an indexed language that its indexer did not cover.
    #[serde(skip)]
    pub uncovered: Vec<String>,
    /// `--component` and `--scope` of the run, as `component web`, `path src/api`.
    #[serde(skip)]
    pub limited_to: Vec<String>,
    /// In diff mode: `a -> b` for component dependencies that exist only
    /// because of the changed lines.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_dependencies: Vec<String>,
}

/// A name used through an import, with the import resolved.
#[derive(Debug, Clone)]
pub struct SymbolRef {
    pub file: std::path::PathBuf,
    pub line: usize,
    /// The node the use is in: the file, or a part of it that only a test build has.
    pub from: crate::lang::Unit,
    pub specifier: String,
    pub name: String,
    /// The method of `name` that is called, with what `forbid_symbol` names it
    /// through: `Shop::Db::Client` and `query`.
    pub method: Option<(String, String)>,
    /// None for a name of the project that no one file defines (a Ruby module
    /// reopened in each file under it): no component owns it.
    pub target: Option<Target>,
}

/// An import between two different components.
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub file: String,
    pub line: usize,
}

/// Checks every internal dependency against the dependency and privacy rules.
/// Every violating import is returned, one per line, see [`dedupe`]. The edges
/// between components feed [`cycles`].
pub fn evaluate(
    config: &Config,
    map: &ComponentMap,
    deps: &[Dep],
) -> (Vec<Violation>, Vec<Edge>, Stats) {
    let mut stats = Stats::default();
    let mut found = Vec::new();
    let mut edges = Vec::new();
    let mut unmapped = BTreeSet::new();
    // The rejected dependencies, judged once every dependency is decided: what
    // re-exports a name one writes, the dependencies of the re-exporter tell.
    let mut rejected = Vec::new();
    // The dependencies that know the lines they stand on, by the components they join.
    let mut by_components: HashMap<(&str, &str), Vec<&Dep>> = HashMap::new();
    for dep in deps {
        let file = slash(&dep.file);
        let external = matches!(dep.target, Target::External(_));
        if external {
            stats.external_deps += 1;
        } else {
            stats.internal_deps += 1;
        }
        let decision = match decision::decide(config, map, dep) {
            Ok(decision) => decision,
            Err(why) => {
                if !external && why == decision::Outside::Unowned {
                    unmapped.insert(file);
                }
                continue;
            }
        };
        if external && map.uses_std(decision.from) && !crate::stdlib::knows(&file) {
            let ext = relpath::extension(&file).to_string();
            if !stats.std_unknown.contains(&ext) {
                stats.std_unknown.push(ext);
            }
        }
        if let Some(to) = decision.to.filter(|to| *to != decision.from) {
            edges.push(Edge {
                from: decision.from.to_string(),
                to: to.to_string(),
                file: file.clone(),
                line: dep.line,
            });
        }
        if let Some(to) = decision.to
            && !dep.mentions.is_empty()
        {
            by_components
                .entry((decision.from, to))
                .or_default()
                .push(dep);
        }
        if decision.grant.is_err() {
            rejected.push((decision, dep));
        }
    }
    for (decision, dep) in rejected {
        let reexport = Reexport::of(map, &decision, dep, &by_components);
        found.extend(violation(&decision, dep, &reexport));
    }
    stats.unmapped_files = unmapped.len();
    (found, edges, stats)
}

/// The components that re-export a name a dependency writes through a module of
/// theirs (`chunk::Block` of `blob`) or through an alias they define, and the lines
/// of theirs behind it.
#[derive(Debug, Default)]
pub struct Reexport {
    pub by: Vec<String>,
    /// Every line of those components that refers to the file of the name through
    /// one of its modules or by the name itself: a change to the re-export changes one.
    pub places: Vec<Place>,
}

impl Reexport {
    /// What re-exports the name `dep` writes, which `decision` rejects: the
    /// component of each alias the line writes, with the definition of every alias
    /// behind `dep`; and each component other than both ends whose module
    /// `dep.through` holds and whose dependencies on the target's component, in
    /// `by_components`, refer to it, by the name itself where a glob import is all
    /// that names the component.
    fn of(
        map: &ComponentMap,
        decision: &Decision,
        dep: &Dep,
        by_components: &HashMap<(&str, &str), Vec<&Dep>>,
    ) -> Self {
        let mut by = BTreeSet::new();
        let mut places = BTreeSet::new();
        let Some(to) = decision.to else {
            return Self::default();
        };
        for aliased in &dep.aliased {
            places.extend(
                aliased
                    .chain
                    .iter()
                    .map(|alias| Place::new(alias.file.clone(), alias.line)),
            );
            if aliased.line == dep.line
                && let Some(component) = aliased
                    .chain
                    .first()
                    .and_then(|written| map.component_of(&written.file))
                    .filter(|component| *component != decision.from && *component != to)
            {
                by.insert(component.to_string());
            }
        }
        let symbols: BTreeSet<&str> = dep
            .mentions
            .iter()
            .filter_map(|mention| mention.symbol.as_deref())
            .collect();
        let candidates: BTreeSet<&str> = dep
            .through
            .files()
            .iter()
            .filter_map(|file| map.component_of(file))
            .filter(|component| *component != decision.from && *component != to)
            .collect();
        let by_name_alone = matches!(dep.through, Through::Glob(_));
        for component in candidates {
            let mentions: Vec<(&Dep, &Mention)> = by_components
                .get(&(component, to))
                .into_iter()
                .flatten()
                .flat_map(|other| other.mentions.iter().map(move |mention| (*other, mention)))
                .filter(|(_, mention)| {
                    mention
                        .symbol
                        .as_deref()
                        .is_none_or(|symbol| symbols.contains(symbol))
                })
                .collect();
            let refers = if by_name_alone {
                mentions.iter().any(|(_, mention)| mention.symbol.is_some())
            } else {
                !mentions.is_empty()
            };
            if refers {
                by.insert(component.to_string());
                places.extend(
                    mentions
                        .iter()
                        .map(|(other, mention)| Place::new(slash(&other.file), mention.line)),
                );
            }
        }
        Self {
            by: by.into_iter().collect(),
            places: places.into_iter().collect(),
        }
    }
}

/// The violation a rejected dependency decision raises at the import behind it,
/// with what re-exports the name it writes, if anything does.
pub fn violation(decision: &Decision, dep: &Dep, reexport: &Reexport) -> Option<Violation> {
    let Decision {
        from,
        to,
        target,
        grant: Err(clause),
        because,
    } = decision
    else {
        return None;
    };
    let file = slash(&dep.file);
    let to = to.unwrap_or_default();
    let reexported = if reexport.by.is_empty() {
        String::new()
    } else {
        format!(", re-exported by {}", reexport.by.join(", "))
    };
    let spec = &format!("{}{reexported}", dep.specifier);
    let message = match clause {
        Clause::External(what) => format!("{from} -> external {target} {what}"),
        Clause::ExcludedTarget => format!(
            "{from} depends on {target}, which `exclude` hides, so no rule checks it; list it in `target_only` if it is generated, in a component or `unowned` if it is part of the project, else drop the dependency"
        ),
        Clause::OtherInstance { here, there } => format!(
            "{here} -> {there}: instances of {from} must not depend on each other (written as {spec})"
        ),
        Clause::SliceMismatch { name, here, there } => {
            format!(
                "{from} -> {to}: <{name}> differs ({here} depends on {there}: {target}{reexported})"
            )
        }
        Clause::Refused(Refusal::Prohibited { .. }) => {
            format!("{from} -> {to} is forbidden (written as {spec})")
        }
        Clause::Refused(Refusal::NotGranted { layers }) => {
            let layers = layers
                .as_ref()
                .map(|why| format!(": {why}"))
                .unwrap_or_default();
            format!("{from} -> {to} is not allowed{layers} (written as {spec})")
        }
        Clause::ImporterNotListed(importers) => format!(
            "{from} -> {to}: {to} is open only to {} (written as {spec})",
            importers.join(", ")
        ),
        Clause::OutsideSurface(held) if held.is_empty() => {
            format!(
                "{from} -> {to}: {target} is not in the public surface of {to} (written as {spec})"
            )
        }
        Clause::OutsideSurface(held) => {
            let names: Vec<&str> = held.iter().map(|h| h.name.as_str()).collect();
            let importers: BTreeSet<&str> = held
                .iter()
                .flat_map(|h| h.importers.iter().map(String::as_str))
                .collect();
            let surfaces = if names.len() == 1 {
                "surface"
            } else {
                "surfaces"
            };
            format!(
                "{from} -> {to}: {target} is {to}'s {} {surfaces}, open to {} (written as {spec})",
                names.join(" and "),
                importers.into_iter().collect::<Vec<_>>().join(", ")
            )
        }
    };
    let mut evidence = vec![Place::new(file.clone(), dep.line)];
    evidence.extend(reexport.places.iter().cloned());
    Some(Violation {
        evidence,
        file,
        line: dep.line,
        message,
        because: because.map(str::to_string),
        from: from.to_string(),
        to: to.to_string(),
        target: target.clone(),
        specifier: dep.specifier.clone(),
        reexported_by: reexport.by.clone(),
        ..Violation::of(clause.rule())
    })
}

/// Component dependencies, as `a -> b`, whose every import sits on a line
/// that `changed` accepts: the edges that did not exist before the change.
pub fn new_dependencies(edges: &[Edge], changed: impl Fn(&str, usize) -> bool) -> Vec<String> {
    let mut all_changed: BTreeMap<(&str, &str), bool> = BTreeMap::new();
    for e in edges {
        let entry = all_changed.entry((&e.from, &e.to)).or_insert(true);
        *entry &= changed(&e.file, e.line);
    }
    all_changed
        .into_iter()
        .filter(|(_, new)| *new)
        .map(|((from, to), _)| format!("{from} -> {to}"))
        .collect()
}

/// Coupling limits: `max_fan_out` counts the components a component depends
/// on, `max_instability` is `out / (in + out)` over the same component edges.
pub fn metrics(config: &Config, edges: &[Edge]) -> Vec<Violation> {
    let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut incoming: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in edges {
        out.entry(&e.from).or_default().insert(&e.to);
        incoming.entry(&e.to).or_default().insert(&e.from);
    }
    let mut found = Vec::new();
    for (name, rule) in &config.rules {
        let Some(targets) = out.get(name.as_str()) else {
            continue;
        };
        let fan_out = targets.len();
        let fan_in = incoming.get(name.as_str()).map_or(0, BTreeSet::len);
        let instability = fan_out as f64 / (fan_in + fan_out) as f64;
        let mut evidence: Vec<Place> = edges
            .iter()
            .filter(|e| e.from == *name)
            .map(|e| Place::new(e.file.clone(), e.line))
            .collect();
        evidence.sort();
        let Place { file, line, .. } = evidence[0].clone();
        let list = targets.iter().copied().collect::<Vec<_>>().join(", ");
        let mut over = |limit: &str, message: String| {
            found.push(Violation {
                file: file.clone(),
                line,
                message,
                because: rule.because.clone(),
                from: name.clone(),
                target: limit.to_string(),
                evidence: evidence.clone(),
                ..Violation::of(RuleId::Metric)
            });
        };
        if let Some(max) = rule.max_fan_out
            && fan_out > max
        {
            over(
                "max_fan_out",
                format!(
                    "{name} depends on {fan_out} components, more than max_fan_out {max}: {list}"
                ),
            );
        }
        if let Some(max) = rule.max_instability
            && instability > max
        {
            over(
                "max_instability",
                format!(
                    "{name} has instability {instability:.2} ({fan_out} out, {fan_in} in), above max_instability {max}"
                ),
            );
        }
    }
    found
}

/// What a directory of a component imports and holds.
#[derive(Default)]
struct Unit {
    /// Checked files, sorted; the first one carries the violations.
    files: BTreeSet<String>,
    components: BTreeSet<String>,
    /// External modules, each with the file that imports it.
    external: BTreeSet<(String, String)>,
}

/// `must_depend_on`, `must_use_external`, `requires_file` and `requires_directive`: every directory
/// that holds checked files of a component must import what the rule lists and
/// contain the files it names. One violation per directory and missing item.
pub fn required(
    root: &Path,
    config: &Config,
    map: &ComponentMap,
    deps: &[Dep],
    paths: &[String],
    all_files: &[String],
) -> Vec<Violation> {
    let parent = |file: &str| relpath::parent(file).to_string();
    let wanted = |from: &str| {
        config.rules.get(from).is_some_and(|r| {
            !(r.must_depend_on.is_empty()
                && r.must_use_external.is_empty()
                && r.requires_file.is_empty()
                && r.requires_directive.is_empty())
        })
    };
    let mut units: BTreeMap<(String, String), Unit> = BTreeMap::new();
    for file in paths {
        if let Some(component) = map.component_of(file).filter(|c| wanted(c)) {
            units
                .entry((component.to_string(), parent(file)))
                .or_default()
                .files
                .insert(file.clone());
        }
    }
    for dep in deps {
        let file = dep.from.key();
        let Some(component) = map.component_of_unit(&dep.from) else {
            continue;
        };
        let Some(unit) = units.get_mut(&(component.to_string(), parent(&file))) else {
            continue;
        };
        match &dep.target {
            Target::Internal(target) => {
                if let Some(to) = map.component_of_unit(target) {
                    unit.components.insert(to.to_string());
                }
            }
            Target::External(module) => {
                unit.external.insert((file.clone(), module.clone()));
            }
        }
    }
    let mut dir_files: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for file in all_files {
        dir_files
            .entry(relpath::parent(file))
            .or_default()
            .push(relpath::file_name(file));
    }
    // Each pattern compiled once, not once per directory; one that does not compile
    // is never present.
    let file_globs: HashMap<&str, Option<globset::GlobMatcher>> = config
        .rules
        .values()
        .flat_map(|rule| &rule.requires_file)
        .map(|w| {
            let glob = globset::Glob::new(w.name())
                .ok()
                .map(|g| g.compile_matcher());
            (w.name(), glob)
        })
        .collect();
    let directives: HashMap<&str, Option<regex::Regex>> = config
        .rules
        .values()
        .flat_map(|rule| &rule.requires_directive)
        .map(|w| (w.name(), regex::Regex::new(w.name()).ok()))
        .collect();
    let mut found = Vec::new();
    for ((component, dir), unit) in &units {
        let rule = &config.rules[component];
        let first = unit.files.first().expect("a unit has files").clone();
        let shown = if dir.is_empty() { "." } else { dir };
        let mut missing = |target: String, message: String, wanted: &Listed| {
            found.push(Violation {
                file: first.clone(),
                line: 1,
                message,
                because: wanted
                    .because()
                    .or(rule.because.as_deref())
                    .map(str::to_string),
                from: component.clone(),
                target,
                evidence: vec![Place::new(first.clone(), 1)],
                ..Violation::of(RuleId::Required)
            });
        };
        for wanted in &rule.must_depend_on {
            if !unit.components.contains(wanted.name()) {
                missing(
                    format!("component {}", wanted.name()),
                    format!(
                        "{shown} ({component}) must depend on {}, and no file in it does",
                        wanted.name()
                    ),
                    wanted,
                );
            }
        }
        for wanted in &rule.must_use_external {
            let used = ComponentMap::external_matches(
                wanted.name(),
                unit.external.iter().map(|(f, m)| (f.as_str(), m.as_str())),
            );
            if !used {
                missing(
                    format!("external {}", wanted.name()),
                    format!(
                        "{shown} ({component}) must use the external module {}, and no file in it does",
                        wanted.name()
                    ),
                    wanted,
                );
            }
        }
        for wanted in &rule.requires_file {
            let names = dir_files.get(dir.as_str()).map_or(&[][..], Vec::as_slice);
            let present = file_globs[wanted.name()]
                .as_ref()
                .is_some_and(|m| names.iter().any(|n| m.is_match(n)));
            if !present {
                missing(
                    format!("file {}", wanted.name()),
                    format!(
                        "{shown} ({component}) must contain a file {}",
                        wanted.name()
                    ),
                    wanted,
                );
            }
        }
        for wanted in &rule.requires_directive {
            let Some(pattern) = &directives[wanted.name()] else {
                continue;
            };
            let present = unit.files.iter().any(|file| {
                std::fs::read_to_string(root.join(file)).is_ok_and(|text| pattern.is_match(&text))
            });
            if !present {
                missing(
                    format!("directive {}", wanted.name()),
                    format!(
                        "{shown} ({component}) must contain a comment or text matching `{}`, and no file in it does",
                        wanted.name()
                    ),
                    wanted,
                );
            }
        }
    }
    found
}

/// `forbid_file`: files of a component whose name or path matches a forbidden glob.
pub fn forbidden_files(config: &Config, map: &ComponentMap, files: &[String]) -> Vec<Violation> {
    let mut found = Vec::new();
    if config
        .rules
        .values()
        .all(|rule| rule.forbid_file.is_empty())
    {
        return found;
    }
    let owners: Vec<Option<&str>> = files.iter().map(|f| map.component_of(f)).collect();
    for (component, rule) in &config.rules {
        if rule.forbid_file.is_empty() {
            continue;
        }
        let matchers: Vec<(&Listed, globset::GlobMatcher)> = rule
            .forbid_file
            .iter()
            .filter_map(|l| Some((l, globset::Glob::new(l.name()).ok()?.compile_matcher())))
            .collect();
        for (file, owner) in files.iter().zip(&owners) {
            if *owner != Some(component.as_str()) {
                continue;
            }
            let name = relpath::file_name(file);
            let Some((listed, _)) = matchers
                .iter()
                .find(|(_, m)| m.is_match(name) || m.is_match(file))
            else {
                continue;
            };
            found.push(Violation {
                file: file.clone(),
                line: 1,
                message: format!(
                    "{component} must not contain {file} (forbid_file {})",
                    listed.name()
                ),
                because: listed
                    .because()
                    .or(rule.because.as_deref())
                    .map(str::to_string),
                from: component.clone(),
                target: file.clone(),
                evidence: vec![Place::new(file.clone(), 1)],
                ..Violation::of(RuleId::File)
            });
        }
    }
    found
}

/// Every type of a component whose name the `defines` of other components
/// claim, and not its own: `OrderRepository` in `web` when `repository`
/// defines `*Repository`.
pub fn naming(
    config: &Config,
    map: &ComponentMap,
    types: &[crate::lang::TypeDef],
) -> Result<Vec<Violation>, anyhow::Error> {
    let mut claims = Vec::new();
    for (name, component) in config
        .components
        .iter()
        .filter(|(_, c)| !c.defines.is_empty())
    {
        for pattern in &component.defines {
            let glob = globset::Glob::new(pattern)
                .map_err(|e| {
                    anyhow::anyhow!("bad pattern `{pattern}` in components.{name}.defines: {e}")
                })?
                .compile_matcher();
            claims.push((name.as_str(), pattern.as_str(), glob));
        }
    }
    let mut found = Vec::new();
    for def in types {
        let Some(home) = map.component_of(&def.file) else {
            continue;
        };
        let owners: Vec<(&str, &str)> = claims
            .iter()
            .filter(|(_, _, glob)| glob.is_match(&def.name))
            .map(|(c, p, _)| (*c, *p))
            .collect();
        if owners.is_empty() || owners.iter().any(|(c, _)| *c == home) {
            continue;
        }
        let (owner, pattern) = owners[0];
        let places: Vec<&str> = owners.iter().map(|(c, _)| *c).collect();
        found.push(Violation {
            file: def.file.clone(),
            line: def.line,
            message: format!(
                "{} is defined in {home}, but types named like {pattern} belong in {}",
                def.name,
                places.join(" or ")
            ),
            because: config.components[owner].because.clone(),
            from: home.to_string(),
            to: owner.to_string(),
            target: def.name.clone(),
            evidence: vec![Place::new(def.file.clone(), def.line)],
            ..Violation::of(RuleId::Naming)
        });
    }
    Ok(found)
}

/// One violation per use: the same key at the same line once, every other line
/// kept, sorted by key and line. The baseline still matches by key alone.
pub fn dedupe(violations: Vec<Violation>) -> Vec<Violation> {
    let mut unique: BTreeMap<(Key, usize), Violation> = BTreeMap::new();
    for v in violations {
        unique.entry((v.key(), v.line)).or_insert(v);
    }
    unique.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canonical_key_stays_canonical() {
        let key = |rule, file: &str| Key {
            rule,
            file: file.into(),
            target: String::new(),
            from: String::new(),
            to: String::new(),
        };
        for (rule, file, want) in [
            (RuleId::Required, "src/a/M.kt", "src/a/"),
            (RuleId::Required, "M.kt", "./"),
            (RuleId::Cycle, "src/a/M.kt", ""),
            (RuleId::Reach, "src/a/M.kt", ""),
            (RuleId::Dependency, "src/a/M.kt", "src/a/M.kt"),
        ] {
            let once = key(rule, file).canonical();
            assert_eq!(once.file, want);
            assert_eq!(once.clone().canonical(), once);
        }
    }

    #[test]
    fn a_type_belongs_in_one_of_the_components_that_claim_its_name() {
        let config: Config = serde_norway::from_str(
            "version: 1
components:
  web: { in: web }
  repo: { in: repo, defines: ['*Repository'] }
  cache: { in: cache, defines: ['*Repository', '*Cache'] }
",
        )
        .unwrap();
        let map = ComponentMap::new(&config).unwrap();
        let def = |file: &str, name: &str| crate::lang::TypeDef {
            file: file.into(),
            line: 3,
            name: name.into(),
        };
        let found = naming(
            &config,
            &map,
            &[
                def("web/a.go", "UserRepository"),
                def("repo/r.go", "OrderRepository"),
                def("cache/c.go", "OrderRepository"),
                def("web/b.go", "Page"),
                def("tools/t.go", "ToolRepository"),
            ],
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "UserRepository is defined in web, but types named like *Repository belong in cache or repo"
        );
        assert_eq!((found[0].file.as_str(), found[0].line), ("web/a.go", 3));
    }

    #[test]
    fn dedupe_keeps_every_line_and_drops_repeats() {
        let at = |line: usize| Violation {
            file: "f".into(),
            line,
            from: "a".into(),
            target: "db.Client".into(),
            ..Violation::of(RuleId::Symbol)
        };
        let lines: Vec<usize> = dedupe(vec![at(9), at(3), at(9)])
            .iter()
            .map(|v| v.line)
            .collect();
        assert_eq!(lines, [3, 9]);
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            file: format!("{from}/x"),
            line: 1,
        }
    }

    fn limits(rule: &str) -> Config {
        serde_norway::from_str(&format!(
            "version: 1
components:
  a: {{ in: a }}
  b: {{ in: b }}
  c: {{ in: c }}
rules:
  a: {{ {rule} }}
"
        ))
        .unwrap()
    }

    #[test]
    fn metrics_report_fan_out_and_instability() {
        let edges = [edge("a", "b"), edge("a", "c"), edge("b", "a")];
        // a: out 2, in 1 -> instability 0.67
        let found = metrics(&limits("max_fan_out: 1, max_instability: 0.5"), &edges);
        let targets: Vec<_> = found.iter().map(|v| v.target.as_str()).collect();
        assert_eq!(targets, ["max_fan_out", "max_instability"]);
        assert!(found[0].message.contains("depends on 2 components"));
        assert!(found[1].message.contains("instability 0.67"));
        assert!(metrics(&limits("max_fan_out: 2, max_instability: 0.7"), &edges).is_empty());
    }

    #[test]
    fn a_dependency_is_new_only_when_all_its_imports_changed() {
        let edges = [
            Edge {
                line: 1,
                ..edge("a", "b")
            },
            Edge {
                line: 2,
                ..edge("a", "b")
            },
            Edge {
                line: 3,
                ..edge("a", "c")
            },
        ];
        // Line 1 of a/x is old, so a -> b existed before; only a -> c is new.
        let got = new_dependencies(&edges, |_, line| line != 1);
        assert_eq!(got, ["a -> c"]);
    }
}
