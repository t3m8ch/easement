//! The constraints of the policy (GLOSSARY.md) that a change to it loosens or
//! tightens: every clause, beyond the decision on a pair of components, that
//! limits what the code may use, must use or must contain.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use super::Sides;
use crate::config::{Component, Config, ForbiddenName, Listed, NoCycles, Rule};
use crate::diagram;
use crate::policy::{Policy, Provenance, Site};

/// Which way a change to a constraint goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// It permits what was refused, or stops requiring what was required.
    Loosened,
    Tightened,
}

/// What happened to an item of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Edit {
    Added,
    Removed,
}

impl Direction {
    /// The way a constraint goes that one side has and the other lacks:
    /// gaining it tightens, losing it loosens.
    fn gained(now: bool) -> Self {
        if now {
            Direction::Tightened
        } else {
            Direction::Loosened
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Direction::Loosened => "loosened",
            Direction::Tightened => "tightened",
        }
    }

    fn opposite(self) -> Self {
        match self {
            Direction::Loosened => Direction::Tightened,
            Direction::Tightened => Direction::Loosened,
        }
    }
}

/// One change to a constraint.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConstraintChange {
    /// The component whose rule or entry holds it; null at the top level.
    pub component: Option<String>,
    pub key: &'static str,
    /// The name, module, glob, component or arrow added or removed, or the
    /// forbidden name whose `allow` changed; null for a value that changed as a
    /// whole.
    pub item: Option<String>,
    /// Whether the item was added or removed; absent for a value that changed
    /// as a whole, and for an `allow` that changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<Edit>,
    /// The value at the merge-base, for a value that changed as a whole
    /// (null when unset), or the `allow` of `item` then (empty when unset).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    pub change: Direction,
    /// What a violation of the constraint shows: the item's own reason, or
    /// else the reason of its rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub because: Option<String>,
    /// Where the policy writes it: the head policy for what it adds or now
    /// holds, the merge-base policy for what it drops.
    pub provenance: Provenance,
}

/// Every change to a constraint, at the top level first, then for each
/// component both policies have, in the order of the head policy. Items are
/// told apart by name, so a list only reordered, or an item whose reason alone
/// changed, is no change; a component one policy lacks is in `components`.
pub fn changes(sides: &Sides) -> Vec<ConstraintChange> {
    let mut diff = Diff {
        sides,
        changes: Vec::new(),
    };
    diff.top_level();
    for name in sides.kept() {
        diff.rule(name);
        diff.component(name);
    }
    diff.changes
}

/// Where a constraint is written: at the top level, in the rule of a
/// component, or in its entry under `components`.
#[derive(Clone, Copy)]
enum At<'a> {
    Top,
    Rule(&'a str),
    Component(&'a str),
}

impl At<'_> {
    fn component(self) -> Option<String> {
        match self {
            At::Top => None,
            At::Rule(name) | At::Component(name) => Some(name.to_string()),
        }
    }

    /// The site of `key`, or of its item at `index`.
    fn site(self, key: &str, index: Option<usize>) -> Site {
        let item = index.map(|i| format!("[{i}]")).unwrap_or_default();
        match self {
            At::Top => Site::in_key(key, format!("{key}{item}")),
            At::Rule(name) => Site::of_entry("rules", name, format!("rules.{name}.{key}{item}")),
            At::Component(name) => {
                Site::of_entry("components", name, format!("components.{name}.{key}{item}"))
            }
        }
    }
}

/// The items of a list as the policy writes them, each with its own reason,
/// and the reason a violation of an item without one shows.
struct Items<'a> {
    names: Vec<(&'a str, Option<&'a str>)>,
    reason: Option<&'a str>,
}

impl<'a> Items<'a> {
    fn listed(list: &'a [Listed], reason: Option<&'a str>) -> Self {
        Self::named(list.iter().map(|l| (l.name(), l.because())), reason)
    }

    /// The names of a `forbid_symbol` list; who each `allow` lets in is
    /// compared by [`Diff::allow`].
    fn forbidden(list: &'a [ForbiddenName], reason: Option<&'a str>) -> Self {
        Self::named(list.iter().map(|l| (l.name(), l.because())), reason)
    }

    fn named(
        names: impl Iterator<Item = (&'a str, Option<&'a str>)>,
        reason: Option<&'a str>,
    ) -> Self {
        Self {
            names: names.collect(),
            reason,
        }
    }

    fn plain(list: &'a [String], reason: Option<&'a str>) -> Self {
        Self {
            names: list.iter().map(|name| (name.as_str(), None)).collect(),
            reason,
        }
    }

    /// The list as a value, for a list set or unset as a whole.
    fn value(&self) -> Value {
        Value::from(self.names.iter().map(|(n, _)| *n).collect::<Vec<_>>())
    }

    /// The first item named `name`, with its index.
    fn find(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|(n, _)| *n == name)
    }
}

/// The changes found so far between the two policies.
struct Diff<'s> {
    sides: &'s Sides<'s>,
    changes: Vec<ConstraintChange>,
}

/// One change before it is placed: what it is, and where to look it up.
struct Found<'a> {
    item: Option<String>,
    before: Option<Value>,
    after: Option<Value>,
    change: Direction,
    because: Option<&'a str>,
    /// Whether the head policy writes it, rather than the merge-base one.
    in_head: bool,
    index: Option<usize>,
}

impl Diff<'_> {
    fn push(&mut self, at: At, key: &'static str, found: Found) {
        let policy = if found.in_head {
            self.sides.head
        } else {
            self.sides.base
        };
        // An item with values is one both sides have, changed within: only an
        // `allow` is written so; an item added or removed carries none.
        let whole = found.before.is_some() || found.after.is_some();
        let edit = (found.item.is_some() && !whole).then_some(if found.in_head {
            Edit::Added
        } else {
            Edit::Removed
        });
        self.changes.push(ConstraintChange {
            component: at.component(),
            key,
            item: found.item,
            edit,
            before: found.before,
            after: found.after,
            change: found.change,
            because: found.because.map(str::to_string),
            provenance: policy.provenance(&at.site(key, found.index)),
        });
    }

    /// The items one list has and the other lacks: an item added goes
    /// `added`, one removed the other way.
    fn items(&mut self, at: At, key: &'static str, then: &Items, now: &Items, added: Direction) {
        for (side, from, other, change) in [
            (true, now, then, added),
            (false, then, now, added.opposite()),
        ] {
            for (index, (name, own)) in from.names.iter().enumerate() {
                if from.find(name) != Some(index) || other.find(name).is_some() {
                    continue;
                }
                self.push(
                    at,
                    key,
                    Found {
                        item: Some(name.to_string()),
                        before: None,
                        after: None,
                        change,
                        because: own.or(from.reason),
                        in_head: side,
                        index: Some(index),
                    },
                );
            }
        }
    }

    /// The names both top-level `forbid_symbol` lists keep whose `allow`
    /// changed: a component let in loosens, one left out tightens, so a change
    /// that does both is listed both ways. No `allow` lets no one in.
    fn allow(&mut self, then: &[ForbiddenName], now: &[ForbiddenName]) {
        for (index, item) in now.iter().enumerate() {
            let Some(old) = then.iter().find(|old| old.name() == item.name()) else {
                continue;
            };
            let (before, after) = (
                old.allow().unwrap_or_default(),
                item.allow().unwrap_or_default(),
            );
            let gained = after.iter().any(|c| !before.contains(c));
            let lost = before.iter().any(|c| !after.contains(c));
            for (changed, change) in [(gained, Direction::Loosened), (lost, Direction::Tightened)] {
                if !changed {
                    continue;
                }
                self.push(
                    At::Top,
                    "forbid_symbol",
                    Found {
                        item: Some(item.name().to_string()),
                        before: Some(Value::from(before.to_vec())),
                        after: Some(Value::from(after.to_vec())),
                        change,
                        because: item.because(),
                        in_head: true,
                        index: Some(index),
                    },
                );
            }
        }
    }

    /// A list that allows, where unset means no limit: setting it tightens,
    /// unsetting it loosens, and between two lists an item added loosens.
    fn allowed(&mut self, at: At, key: &'static str, then: Option<Items>, now: Option<Items>) {
        match (then, now) {
            (None, None) => {}
            (Some(then), Some(now)) => self.items(at, key, &then, &now, Direction::Loosened),
            (then, now) => {
                let value =
                    |items: &Option<Items>| items.as_ref().map_or(Value::Null, Items::value);
                let reason = now.as_ref().or(then.as_ref()).and_then(|i| i.reason);
                self.push(
                    at,
                    key,
                    Found {
                        item: None,
                        before: Some(value(&then)),
                        after: Some(value(&now)),
                        change: Direction::gained(now.is_some()),
                        because: reason,
                        in_head: now.is_some(),
                        index: None,
                    },
                );
            }
        }
    }

    /// A flag whose turning on goes `on`.
    fn flag(
        &mut self,
        at: At,
        key: &'static str,
        (then, now): (bool, bool),
        on: Direction,
        because: (Option<&str>, Option<&str>),
    ) {
        if then == now {
            return;
        }
        self.push(
            at,
            key,
            Found {
                item: None,
                before: Some(Value::from(then)),
                after: Some(Value::from(now)),
                change: if now { on } else { on.opposite() },
                because: if now { because.1 } else { because.0 },
                in_head: now,
                index: None,
            },
        );
    }

    /// A limit: a higher one or none loosens, a lower one or a new one tightens.
    fn limit<T: PartialOrd + Copy + Into<Value>>(
        &mut self,
        at: At,
        key: &'static str,
        (then, now): (Option<T>, Option<T>),
        because: (Option<&str>, Option<&str>),
    ) {
        let change = match (then, now) {
            (None, Some(_)) | (Some(_), None) => Direction::gained(now.is_some()),
            (Some(a), Some(b)) if b > a => Direction::Loosened,
            (Some(a), Some(b)) if b < a => Direction::Tightened,
            _ => return,
        };
        let value = |v: Option<T>| v.map_or(Value::Null, Into::into);
        self.push(
            at,
            key,
            Found {
                item: None,
                before: Some(value(then)),
                after: Some(value(now)),
                change,
                because: if now.is_some() { because.1 } else { because.0 },
                in_head: now.is_some(),
                index: None,
            },
        );
    }

    fn top_level(&mut self) {
        let (base, head) = (&self.sides.base.config, &self.sides.head.config);
        // Every key, so that a new one cannot be left out.
        let Config {
            // The format, not a clause.
            version: _,
            // Compared per component below; one added or removed is in `components`.
            components: _,
            rules: _,
            // The decision on pairs of components: `permissions`.
            common: _,
            accept_augmentations,
            layers: _,
            // Which files are judged as the source of a dependency: the debt section.
            exclude: _,
            target_only: _,
            // The debt section compares its entries, not where they are kept.
            baseline: _,
            uncovered_ok: _,
            warn: _,
            // What is scanned: `rescanned` names them, and the delta shows their effect.
            reach: _,
            indexers: _,
            tests: _,
            packages: _,
            // The policy examples section.
            examples: _,
            forbid_symbol,
            no_cycles,
            strict,
            unowned,
            entry,
            diagram,
            manifests,
        } = head;
        let none = (None, None);
        self.items(
            At::Top,
            "forbid_symbol",
            &Items::forbidden(&base.forbid_symbol, None),
            &Items::forbidden(forbid_symbol, None),
            Direction::Tightened,
        );
        let accepted = |items: &[crate::config::AugmentationAcceptance]| {
            items
                .iter()
                .map(|a| {
                    format!(
                        "{} -> {} in {}: {}",
                        a.consumers.join(", "),
                        a.module,
                        a.declaration,
                        a.because
                    )
                })
                .collect::<Vec<_>>()
        };
        let before = accepted(&base.accept_augmentations);
        let after = accepted(accept_augmentations);
        self.items(
            At::Top,
            "accept_augmentations",
            &Items::plain(&before, None),
            &Items::plain(&after, None),
            Direction::Loosened,
        );
        self.allow(&base.forbid_symbol, forbid_symbol);
        self.no_cycles(&base.no_cycles, no_cycles);
        self.flag(
            At::Top,
            "strict",
            (base.strict, *strict),
            Direction::Tightened,
            none,
        );
        self.items(
            At::Top,
            "unowned",
            &Items::plain(&base.unowned, None),
            &Items::plain(unowned, None),
            Direction::Loosened,
        );
        // With an entry the `unused` check runs: it starts unset, as no list.
        let entries = |list| Some(Items::plain(list, None)).filter(|i| !i.names.is_empty());
        self.allowed(At::Top, "entry", entries(&base.entry), entries(entry));
        self.flag(
            At::Top,
            "manifests",
            (base.manifests, *manifests),
            Direction::Tightened,
            none,
        );
        self.diagram(base.diagram.as_ref(), diagram.as_ref());
    }

    /// The components both policies have that enter or leave `no_cycles`.
    fn no_cycles(&mut self, then: &NoCycles, now: &NoCycles) {
        let index = |cycles: &NoCycles, name: &str| match cycles {
            NoCycles::Among(list) => list.iter().position(|c| c == name),
            NoCycles::Off | NoCycles::All(_) => None,
        };
        let kept: Vec<&str> = self.sides.kept().collect();
        for name in kept {
            let (covered, covers) = (then.covers(name), now.covers(name));
            if covered == covers {
                continue;
            }
            self.push(
                At::Top,
                "no_cycles",
                Found {
                    item: Some(name.to_string()),
                    before: None,
                    after: None,
                    change: Direction::gained(covers),
                    because: None,
                    in_head: covers,
                    index: index(if covers { now } else { then }, name),
                },
            );
        }
    }

    /// A diagram set or unset as a whole, or the arrows, as pairs of
    /// components, that one diagram draws and the other does not. An arrow
    /// of a component the head policy drops goes with the component, which
    /// `components` lists; one to a component it adds is listed. A diagram
    /// that cannot be read is not guessed at: the review names it.
    fn diagram(&mut self, then: Option<&PathBuf>, now: Option<&PathBuf>) {
        let path =
            |p: Option<&PathBuf>| p.map_or(Value::Null, |p| Value::from(crate::relpath::slash(p)));
        if then.is_some() != now.is_some() {
            self.push(
                At::Top,
                "diagram",
                Found {
                    item: None,
                    before: Some(path(then)),
                    after: Some(path(now)),
                    change: Direction::gained(now.is_some()),
                    because: None,
                    in_head: now.is_some(),
                    index: None,
                },
            );
            return;
        }
        let (Ok(Some(then)), Ok(Some(now))) = (
            diagram::drawn(self.sides.base),
            diagram::drawn(self.sides.head),
        ) else {
            return;
        };
        for (policy, from, other, change, edit) in [
            (
                self.sides.head,
                &now,
                &then,
                Direction::Loosened,
                Edit::Added,
            ),
            (
                self.sides.base,
                &then,
                &now,
                Direction::Tightened,
                Edit::Removed,
            ),
        ] {
            let components = &self.sides.head.config.components;
            let kept = |(a, b): &&(String, String)| {
                components.contains_key(a) && components.contains_key(b)
            };
            for (a, b) in from.difference(other).filter(kept) {
                self.changes.push(ConstraintChange {
                    component: None,
                    key: "diagram",
                    item: Some(format!("{a} -> {b}")),
                    edit: Some(edit),
                    before: None,
                    after: None,
                    change,
                    because: None,
                    provenance: drawn_in(policy),
                });
            }
        }
    }

    /// The rule of a component both policies have; none is a rule with
    /// nothing in it.
    fn rule(&mut self, name: &str) {
        let empty = Rule::default();
        let then = self.sides.base.config.rules.get(name).unwrap_or(&empty);
        let now = self.sides.head.config.rules.get(name).unwrap_or(&empty);
        let at = At::Rule(name);
        // Every key, so that a new one cannot be left out.
        let Rule {
            // The decision on pairs of components: `permissions`.
            may_depend_on: _,
            must_not_depend_on: _,
            must_not_reach: _,
            // The reason of each change below.
            because: _,
            forbid_symbol,
            forbid_external,
            may_use_external,
            forbid_file,
            must_depend_on,
            must_use_external,
            requires_file,
            requires_directive,
            max_fan_out,
            max_instability,
        } = now;
        let reasons = (then.because.as_deref(), now.because.as_deref());
        // The lists that forbid or require: an item added tightens.
        self.items(
            at,
            "forbid_symbol",
            &Items::forbidden(&then.forbid_symbol, reasons.0),
            &Items::forbidden(forbid_symbol, reasons.1),
            Direction::Tightened,
        );
        for (key, before, after) in [
            ("forbid_external", &then.forbid_external, forbid_external),
            ("forbid_file", &then.forbid_file, forbid_file),
            ("must_depend_on", &then.must_depend_on, must_depend_on),
            (
                "must_use_external",
                &then.must_use_external,
                must_use_external,
            ),
            ("requires_file", &then.requires_file, requires_file),
            (
                "requires_directive",
                &then.requires_directive,
                requires_directive,
            ),
        ] {
            self.items(
                at,
                key,
                &Items::listed(before, reasons.0),
                &Items::listed(after, reasons.1),
                Direction::Tightened,
            );
        }
        self.allowed(
            at,
            "may_use_external",
            then.may_use_external
                .as_deref()
                .map(|l| Items::listed(l, reasons.0)),
            may_use_external
                .as_deref()
                .map(|l| Items::listed(l, reasons.1)),
        );
        self.limit(at, "max_fan_out", (then.max_fan_out, *max_fan_out), reasons);
        self.limit(
            at,
            "max_instability",
            (then.max_instability, *max_instability),
            reasons,
        );
    }

    /// The entry of a component both policies have.
    fn component(&mut self, name: &str) {
        let then = &self.sides.base.config.components[name];
        let now = &self.sides.head.config.components[name];
        let at = At::Component(name);
        // Every key, so that a new one cannot be left out.
        let Component {
            // Which files the component owns: `ownership`.
            patterns: _,
            except: _,
            priority: _,
            part: _,
            // Who may import which files: `surfaces`.
            public: _,
            allow_importers: _,
            surfaces: _,
            // The reason of each change below.
            because: _,
            // Who to ask, not a clause.
            owner: _,
            public_symbols,
            isolated,
            defines,
            optional,
        } = now;
        let reasons = (then.because.as_deref(), now.because.as_deref());
        self.allowed(
            at,
            "public_symbols",
            then.public_symbols
                .as_deref()
                .map(|l| Items::plain(l, reasons.0)),
            public_symbols
                .as_deref()
                .map(|l| Items::plain(l, reasons.1)),
        );
        // An import between instances shows the reason of the component, or
        // else that of its rule.
        self.flag(
            at,
            "isolated",
            (then.isolated, *isolated),
            Direction::Tightened,
            (
                reasons.0.or(self.sides.base.config.reason(name)),
                reasons.1.or(self.sides.head.config.reason(name)),
            ),
        );
        self.items(
            at,
            "defines",
            &Items::plain(&then.defines, reasons.0),
            &Items::plain(defines, reasons.1),
            Direction::Tightened,
        );
        self.flag(
            at,
            "optional",
            (then.optional, *optional),
            Direction::Loosened,
            (None, None),
        );
    }
}

impl ConstraintChange {
    /// The change in one line, as the text lists it:
    /// ``web.forbid_symbol: removed `tags.DeleteIn` (loosened)``.
    pub(super) fn line(&self) -> String {
        let key = match &self.component {
            Some(component) => format!("{component}.{}", self.key),
            None => self.key.to_string(),
        };
        let what = match (&self.item, self.edit) {
            (Some(item), Some(edit)) => {
                let verb = match edit {
                    Edit::Added => "added",
                    Edit::Removed => "removed",
                };
                let noun = if self.key == "diagram" { " arrow" } else { "" };
                format!("{verb}{noun} `{item}`")
            }
            // Who may use a forbidden name.
            (Some(item), None) => format!(
                "`{item}` allows {} (was {})",
                shown(self.after.as_ref()),
                shown(self.before.as_ref())
            ),
            _ => format!(
                "{} (was {})",
                shown(self.after.as_ref()),
                shown(self.before.as_ref())
            ),
        };
        format!("{key}: {what} ({})", self.change.as_str())
    }
}

/// A value as the text shows it: `unset` for none, lists and paths quoted.
fn shown(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "unset".to_string(),
        Some(Value::Array(items)) if items.is_empty() => "none".to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| format!("`{}`", i.as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::String(path)) => format!("`{path}`"),
        Some(other) => other.to_string(),
    }
}

/// Where an arrow is drawn: the diagram of `policy`, under `diagram`.
fn drawn_in(policy: &Policy) -> Provenance {
    Provenance {
        field: "diagram".to_string(),
        input: policy
            .diagram()
            .ok()
            .flatten()
            .map(|input| policy.input_name(input)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{compare, compare_with};
    use super::*;

    const TWO: &str = "version: 1
components:
  web: { in: web }
  db: { in: db }
";

    /// Each change as `component.key: item (direction)`, short to compare.
    fn said(changes: &[ConstraintChange]) -> Vec<String> {
        changes
            .iter()
            .map(|c| {
                let key = match &c.component {
                    Some(component) => format!("{component}.{}", c.key),
                    None => c.key.to_string(),
                };
                let what = match (&c.item, &c.before, &c.after) {
                    (Some(item), _, _) => item.clone(),
                    (None, before, after) => format!(
                        "{} -> {}",
                        before.as_ref().unwrap(),
                        after.as_ref().unwrap()
                    ),
                };
                format!("{key}: {what} ({:?})", c.change)
            })
            .collect()
    }

    #[test]
    fn an_item_added_to_a_list_that_forbids_tightens_and_one_removed_loosens() {
        let base = format!(
            "{TWO}forbid_symbol: [{{ name: tags.DeleteIn, because: tags are kept forever }}]\n"
        );
        let head = format!(
            "{TWO}rules:\n  web: {{ forbid_external: [lodash], because: pages stay small }}\n"
        );
        let changes = compare(&base, &head, changes);
        assert_eq!(
            said(&changes),
            [
                "forbid_symbol: tags.DeleteIn (Loosened)",
                "web.forbid_external: lodash (Tightened)",
            ]
        );
        let (removed, added) = (&changes[0], &changes[1]);
        assert_eq!(removed.because.as_deref(), Some("tags are kept forever"));
        assert_eq!(removed.provenance.field, "forbid_symbol[0]");
        // A plain item gives the reason of its rule.
        assert_eq!(added.because.as_deref(), Some("pages stay small"));
        assert_eq!(added.provenance.field, "rules.web.forbid_external[0]");
        assert_eq!(added.provenance.input.as_deref(), Some("easement.yml"));
    }

    #[test]
    fn a_component_let_into_a_forbidden_name_loosens_and_one_left_out_tightens() {
        let three = TWO.replace("db: { in: db }", "db: { in: db }\n  jobs: { in: jobs }");
        let item = |allow: &str| {
            format!(
                "{three}forbid_symbol:\n  - tags.Rename\n  - {{ name: tags.DeleteIn{allow}, because: a use case }}\n"
            )
        };
        let (plain, web, db) = (item(""), item(", allow: [web]"), item(", allow: [db]"));
        let found = compare(&plain, &web, changes);
        assert_eq!(said(&found), ["forbid_symbol: tags.DeleteIn (Loosened)"]);
        let wider = &found[0];
        assert_eq!(wider.edit, None);
        assert_eq!(wider.before, Some(serde_json::json!([])));
        assert_eq!(wider.after, Some(serde_json::json!(["web"])));
        assert_eq!(wider.because.as_deref(), Some("a use case"));
        assert_eq!(wider.provenance.field, "forbid_symbol[1]");
        assert_eq!(
            wider.line(),
            "forbid_symbol: `tags.DeleteIn` allows `web` (was none) (loosened)"
        );
        // One component let in and another left out: both ways at once.
        assert_eq!(
            said(&compare(&web, &db, changes)),
            [
                "forbid_symbol: tags.DeleteIn (Loosened)",
                "forbid_symbol: tags.DeleteIn (Tightened)",
            ]
        );
        assert_eq!(
            said(&compare(&db, &plain, changes)),
            ["forbid_symbol: tags.DeleteIn (Tightened)"]
        );
        // Only reordered: no change.
        let both = item(", allow: [web, db]");
        assert!(compare(&both, &item(", allow: [db, web]"), changes).is_empty());
    }

    #[test]
    fn a_list_that_allows_tightens_when_set_and_loosens_with_each_item_added() {
        let base = format!("{TWO}rules:\n  web: {{ max_fan_out: 3 }}\n");
        let set = format!("{TWO}rules:\n  web: {{ max_fan_out: 5, may_use_external: [react] }}\n");
        assert_eq!(
            said(&compare(&base, &set, changes)),
            [
                "web.may_use_external: null -> [\"react\"] (Tightened)",
                "web.max_fan_out: 3 -> 5 (Loosened)",
            ]
        );
        let wider = set.replace("[react]", "[react, lodash]");
        assert_eq!(
            said(&compare(&set, &wider, changes)),
            ["web.may_use_external: lodash (Loosened)"]
        );
        // Unset again, the rule entry dropped whole: no limit at all.
        assert_eq!(
            said(&compare(&wider, TWO, changes)),
            [
                "web.may_use_external: [\"react\",\"lodash\"] -> null (Loosened)",
                "web.max_fan_out: 5 -> null (Loosened)",
            ]
        );
    }

    #[test]
    fn flags_entries_and_public_symbols_go_the_way_each_key_means() {
        let base = TWO.replace("db: { in: db }", "db: { in: db/*, public_symbols: [Open] }");
        let head = TWO.replace(
            "db: { in: db }",
            "db: { in: db/*, optional: true, isolated: true }",
        ) + "strict: true\nentry: [cmd/**]\nmanifests: true\nunowned: [scripts/**]\n";
        assert_eq!(
            said(&compare(&base, &head, changes)),
            [
                "strict: false -> true (Tightened)",
                "unowned: scripts/** (Loosened)",
                "entry: null -> [\"cmd/**\"] (Tightened)",
                "manifests: false -> true (Tightened)",
                "db.public_symbols: [\"Open\"] -> null (Loosened)",
                "db.isolated: false -> true (Tightened)",
                "db.optional: false -> true (Loosened)",
            ]
        );
    }

    #[test]
    fn no_cycles_tightens_for_each_component_that_enters_it() {
        let base = format!("{TWO}no_cycles: [web]\n");
        let head = format!("{TWO}no_cycles: true\n");
        let changes = compare(&base, &head, changes);
        assert_eq!(said(&changes), ["no_cycles: db (Tightened)"]);
        assert_eq!(changes[0].provenance.field, "no_cycles");
        let back = compare(&head, &base, super::changes);
        assert_eq!(said(&back), ["no_cycles: db (Loosened)"]);
    }

    #[test]
    fn a_reordered_list_or_a_new_reason_alone_changes_no_constraint() {
        let base = format!(
            "{TWO}rules:\n  web: {{ forbid_external: [a, {{ name: b, because: old }}] }}\n"
        );
        let head = format!(
            "{TWO}rules:\n  web: {{ forbid_external: [{{ name: b, because: new }}, a], because: new }}\n"
        );
        assert_eq!(said(&compare(&base, &head, changes)), Vec::<String>::new());
    }

    const DRAWN: &str = "```mermaid\ngraph TD\n  web --> db\n```\n";
    const UNDRAWN: &str = "```mermaid\ngraph TD\n  web\n  db\n```\n";

    #[test]
    fn a_diagram_arrow_added_loosens_and_a_diagram_set_tightens() {
        let files = [
            ("old.md", UNDRAWN),
            ("new.md", DRAWN),
            ("bare.md", "no graph\n"),
        ];
        let old = format!("{TWO}diagram: old.md\n");
        let new = format!("{TWO}diagram: new.md\n");
        let changes = compare_with(&files, &old, &new, changes);
        assert_eq!(said(&changes), ["diagram: web -> db (Loosened)"]);
        assert_eq!(changes[0].provenance.input.as_deref(), Some("new.md"));
        assert_eq!(
            said(&compare_with(&files, TWO, &old, super::changes)),
            ["diagram: null -> \"old.md\" (Tightened)"]
        );

        // An arrow to a component the base policy lacks is listed too.
        let api = [("api.md", "```mermaid\ngraph TD\n  web --> api\n```\n")];
        let with_api = TWO.replace(
            "  db: { in: db }\n",
            "  db: { in: db }\n  api: { in: api }\n",
        );
        assert_eq!(
            said(&compare_with(
                &[files.as_slice(), &api].concat(),
                &old,
                &format!("{with_api}diagram: api.md\n"),
                super::changes
            )),
            ["diagram: web -> api (Loosened)"]
        );

        // The arrows of a component the head policy drops go with it: the
        // component is in `components`, not each of its arrows.
        assert_eq!(
            said(&compare_with(
                &[files.as_slice(), &api].concat(),
                &format!("{with_api}diagram: api.md\n"),
                &old,
                super::changes
            )),
            Vec::<String>::new()
        );

        // No arrow is guessed from a diagram that cannot be read.
        let bare = format!("{TWO}diagram: bare.md\n");
        assert_eq!(
            said(&compare_with(&files, &bare, &new, super::changes)),
            Vec::<String>::new()
        );
    }
}
