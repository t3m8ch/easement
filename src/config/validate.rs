//! What makes a config wrong: a check of every key against the others, so a
//! mistake fails when the file is read, with the path to the field and the
//! closest name.

use super::*;

impl Config {
    /// Rejects a config whose keys contradict each other or name what does not exist.
    pub(super) fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported config version {}, expected 1", self.version);
        }
        if self.components.is_empty() {
            bail!("config declares no components");
        }
        for name in &self.warn {
            match RuleId::ALL.into_iter().find(|r| r.as_str() == name) {
                Some(rule) if rule.is_config_rule() => bail!(
                    "`{name}` is about the config, so it cannot be a warning: fix the config instead"
                ),
                Some(_) => {}
                None => {
                    let known: Vec<&str> = RuleId::ALL
                        .into_iter()
                        .filter(|r| !r.is_config_rule())
                        .map(RuleId::as_str)
                        .collect();
                    bail!(
                        "unknown rule `{name}` in warn; one of: {}",
                        known.join(", ")
                    );
                }
            }
        }
        crate::lang::registry::check_settings(&self.indexers).map_err(anyhow::Error::msg)?;
        let known: BTreeSet<&str> = self.components.keys().map(String::as_str).collect();
        let check = |name: &str, place: &str| -> Result<()> {
            if !known.contains(name) {
                return Err(unknown_component(
                    name,
                    &format!(" in {place}"),
                    known.iter().copied(),
                ));
            }
            Ok(())
        };
        for accept in &self.accept_augmentations {
            if accept.consumers.is_empty()
                || accept.module.trim().is_empty()
                || accept.because.trim().is_empty()
            {
                bail!("accept_augmentations requires consumers, module and a nonempty because");
            }
            if accept.module.starts_with(['.', '/']) || accept.module.contains(['*', '?']) {
                bail!("accept_augmentations.module must be an exact external module");
            }
            let path = &accept.declaration;
            if path.is_empty()
                || path.starts_with('/')
                || path.contains(['*', '?', '[', '\\', ':'])
                || path
                    .split('/')
                    .any(|p| p == ".." || p == "." || p.is_empty())
            {
                bail!("accept_augmentations.declaration must be an exact project-relative path");
            }
            for consumer in &accept.consumers {
                check(consumer, "accept_augmentations.consumers")?;
            }
        }
        for name in &self.common {
            check(name, "common")?;
        }
        for symbol in self.forbid_symbol.iter().map(ForbiddenName::name) {
            if !symbol.contains('.') {
                bail!("forbid_symbol `{symbol}` must look like `package.Name`");
            }
        }
        for (from, rule) in &self.rules {
            check(from, "rules")?;
            for to in rule.may_depend_on.iter().map(Listed::name) {
                check(to, &format!("rules.{from}.may_depend_on"))?;
            }
            for to in rule.must_not_depend_on.iter().map(Listed::name) {
                check(to, &format!("rules.{from}.must_not_depend_on"))?;
                if rule.may_depend_on.iter().any(|c| c.name() == to) {
                    bail!("rules.{from} both allows and forbids `{to}`");
                }
            }
            for to in rule.must_not_reach.iter().map(Listed::name) {
                check(to, &format!("rules.{from}.must_not_reach"))?;
                if to == from {
                    bail!("rules.{from}.must_not_reach names `{from}` itself");
                }
                if rule.may_depend_on.iter().any(|c| c.name() == to) {
                    bail!("rules.{from} both allows `{to}` and forbids reaching it");
                }
            }
            for to in rule.must_depend_on.iter().map(Listed::name) {
                check(to, &format!("rules.{from}.must_depend_on"))?;
            }
            if let Some(name) = rule
                .may_use_external
                .iter()
                .flatten()
                .chain(&rule.forbid_external)
                .map(Listed::name)
                .find(|n| n.starts_with('$') && *n != STD)
            {
                bail!(
                    "rules.{from}: unknown `{name}`; the only `$` name is `$std`, the standard library"
                );
            }
            if rule.must_use_external.iter().any(|l| l.name() == STD) {
                bail!(
                    "rules.{from}.must_use_external: `$std` works only in may_use_external and forbid_external"
                );
            }
            for pattern in rule
                .must_use_external
                .iter()
                .chain(&rule.requires_file)
                .chain(&rule.forbid_file)
                .map(Listed::name)
            {
                globset::Glob::new(pattern)
                    .with_context(|| format!("bad pattern `{pattern}` in rules.{from}"))?;
            }
            for listed in &rule.forbid_symbol {
                let symbol = listed.name();
                if !symbol.contains('.') {
                    bail!("rules.{from}.forbid_symbol `{symbol}` must look like `package.Name`");
                }
                // A component's own list binds only its files, so it has no one to let in.
                if listed.allow().is_some() {
                    bail!(
                        "rules.{from}.forbid_symbol `{symbol}` has `allow`, which works only in the top-level forbid_symbol"
                    );
                }
            }
            for pattern in rule.requires_directive.iter().map(Listed::name) {
                regex::Regex::new(pattern).with_context(|| {
                    format!("bad regular expression `{pattern}` in rules.{from}.requires_directive")
                })?;
            }
            if rule
                .max_instability
                .is_some_and(|i| !(0.0..=1.0).contains(&i))
            {
                bail!("rules.{from}.max_instability must be between 0 and 1");
            }
        }
        self.validate_forbid_symbol_allow(&check)?;
        // Named placeholders belong to `in` patterns only.
        let plain = |place: &str, patterns: Vec<&str>| -> Result<()> {
            if let Some(p) = patterns.iter().find(|p| p.contains('<')) {
                bail!("`{p}` in {place}: a placeholder like `<feature>` works only in `in`");
            }
            Ok(())
        };
        plain("exclude", self.exclude.iter().map(String::as_str).collect())?;
        plain(
            "target_only",
            self.target_only.iter().map(String::as_str).collect(),
        )?;
        plain("unowned", self.unowned.iter().map(String::as_str).collect())?;
        plain("entry", self.entry.iter().map(String::as_str).collect())?;
        plain(
            "uncovered_ok",
            self.uncovered_ok
                .iter()
                .flat_map(Accepted::patterns)
                .collect(),
        )?;
        for (name, component) in &self.components {
            if component.patterns.as_vec().is_empty() {
                bail!("component `{name}` has no `in` patterns");
            }
            if component.part.is_some() && component.isolated {
                bail!("component `{name}` owns a `part`, which cannot be `isolated`");
            }
            for pattern in component.patterns.as_vec() {
                crate::components::glob_and_placeholders(pattern)
                    .with_context(|| format!("components.{name}.in"))?;
            }
            plain(
                &format!("components.{name}.public"),
                component
                    .public
                    .as_ref()
                    .map(Patterns::as_vec)
                    .unwrap_or_default(),
            )?;
            for pattern in &component.defines {
                globset::Glob::new(pattern).with_context(|| {
                    format!("bad pattern `{pattern}` in components.{name}.defines")
                })?;
            }
            plain(
                &format!("components.{name}.except"),
                component
                    .except
                    .as_ref()
                    .map(Patterns::as_vec)
                    .unwrap_or_default(),
            )?;
            if component
                .public
                .as_ref()
                .is_some_and(|p| p.as_vec().is_empty())
            {
                bail!("component `{name}` has an empty `public` list");
            }
            for importer in component.allow_importers.iter().flatten() {
                check(importer, &format!("components.{name}.allow_importers"))?;
            }
            for (surface_name, surface) in &component.surfaces {
                let place = format!("components.{name}.surfaces.{surface_name}");
                let paths = surface.patterns.as_vec();
                if paths.is_empty() {
                    bail!("{place} has no `in` patterns");
                }
                plain(&place, paths.clone())?;
                let inside = component.patterns.as_vec();
                let except = component
                    .except
                    .as_ref()
                    .map(Patterns::as_vec)
                    .unwrap_or_default();
                for path in paths {
                    if !crate::components::lies_inside(name, &inside, &except, path)? {
                        bail!(
                            "{place}: `{path}` is outside the component, whose `in` is {}",
                            inside.join(", ")
                        );
                    }
                }
                if surface.importers.is_empty() {
                    bail!("{place} has no importers; list the components that may use it");
                }
                for importer in &surface.importers {
                    check(importer, &format!("{place}.importers"))?;
                    if let Some(admitted) = &component.allow_importers
                        && !admitted.contains(importer)
                    {
                        bail!(
                            "{place} opens to `{importer}`, which allow_importers does not admit; add it there or drop it from the surface"
                        );
                    }
                }
            }
            if component.isolated
                && component
                    .patterns
                    .as_vec()
                    .iter()
                    .any(|p| p.contains("**") || !(p.contains('*') || p.contains('<')))
            {
                bail!(
                    "component `{name}` is isolated, so every `in` pattern needs a `*` or a placeholder like `<page>` to tell the instances apart, and no `**`"
                );
            }
        }
        let mut layers = BTreeSet::new();
        if let Some(Layer::Siblings(names)) = self
            .layers
            .iter()
            .find(|l| matches!(l, Layer::Siblings(n) if n.len() < 2))
        {
            bail!(
                "layers: `{names:?}` lists fewer than two siblings; write a lone layer as its name"
            );
        }
        for name in self.layers.iter().flat_map(Layer::names) {
            check(name, "layers")?;
            if !layers.insert(name) {
                bail!("component `{name}` is listed twice in layers");
            }
        }
        if let NoCycles::Among(list) = &self.no_cycles {
            for name in list {
                check(name, "no_cycles")?;
            }
        }
        for (i, example) in self.examples.iter().enumerate() {
            let at = format!("examples[{i}] ({})", example.from);
            match (&example.to, &example.external, example.rule) {
                (Some(_), Some(_), _) | (None, None, _) => {
                    bail!("{at} needs exactly one of `to` and `external`")
                }
                (_, _, Some(rule)) if example.expect == Expect::Allowed => bail!(
                    "{at} expects allowed, so no rule `{}` can reject it; drop `rule` or expect rejected",
                    rule.id().as_str()
                ),
                (Some(_), None, Some(ExampleRule::External)) => {
                    bail!("{at}: only an `external` module can be rejected by rule `external`")
                }
                (None, Some(_), Some(rule)) if rule != ExampleRule::External => bail!(
                    "{at}: an `external` module is rejected only by rule `external`, never `{}`",
                    rule.id().as_str()
                ),
                _ => {}
            }
        }
        Ok(())
    }

    /// Rejects an `allow` of the top-level `forbid_symbol` that lets no one in,
    /// names what is no component, lets in a component whose own rule forbids
    /// the name, or sits on a name another item can name too, spelled alike or
    /// under a longer qualifier, where which item holds is unclear.
    fn validate_forbid_symbol_allow(
        &self,
        check: &impl Fn(&str, &str) -> Result<()>,
    ) -> Result<()> {
        for (i, listed) in self.forbid_symbol.iter().enumerate() {
            let Some(allowed) = listed.allow() else {
                continue;
            };
            let place = format!("forbid_symbol[{i}].allow");
            if allowed.is_empty() {
                bail!("{place} lists no component; leave the key out instead");
            }
            let symbol = listed.name();
            for component in allowed {
                check(component, &place)?;
                let own = self.rules.get(component).and_then(|rule| {
                    rule.forbid_symbol
                        .iter()
                        .map(ForbiddenName::name)
                        .find(|own| crate::symbols::may_name_the_same(own, symbol))
                });
                if let Some(own) = own {
                    let spelled = if own == symbol {
                        String::new()
                    } else {
                        format!(" as `{own}`")
                    };
                    bail!(
                        "{place} lets `{component}` use `{symbol}`, which rules.{component}.forbid_symbol forbids{spelled}"
                    );
                }
            }
        }
        for (i, first) in self.forbid_symbol.iter().enumerate() {
            for (j, second) in self.forbid_symbol.iter().enumerate().skip(i + 1) {
                if (first.allow().is_some() || second.allow().is_some())
                    && crate::symbols::may_name_the_same(first.name(), second.name())
                {
                    bail!(
                        "forbid_symbol[{i}] `{}` and forbid_symbol[{j}] `{}` can name the same symbol, and one of them has `allow`, so who may use it is ambiguous: list it once",
                        first.name(),
                        second.name()
                    );
                }
            }
        }
        Ok(())
    }
}

/// The error for `name`, which is no component in `known`: `place` says where it
/// was found (` in common`, or nothing), and the error names the component
/// `name` is likely a typo of (a few edits away, a transposition counting as one).
pub(super) fn unknown_component<'a>(
    name: &str,
    place: &str,
    known: impl IntoIterator<Item = &'a str>,
) -> anyhow::Error {
    match closest(name, known) {
        Some(k) => anyhow!("unknown component `{name}`{place}; did you mean `{k}`?"),
        None => anyhow!("unknown component `{name}`{place}"),
    }
}

/// The name in `known` that `name` is likely a typo of: a few edits away, a
/// transposition counting as one.
pub fn closest<'a>(name: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (name.chars().count() / 3).max(1);
    known
        .into_iter()
        .map(|k| (strsim::damerau_levenshtein(name, k), k))
        .filter(|(distance, _)| *distance <= limit)
        .min()
        .map(|(_, k)| k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Config> {
        let config: Config = serde_norway::from_str(yaml)?;
        let config = config.with_tests()?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn an_unknown_component_suggests_the_closest_name() {
        let yaml = |common: &str| {
            format!(
                "version: 1\ncomponents:\n  billing: {{ in: b }}\n  shared: {{ in: s }}\ncommon: [{common}]\n"
            )
        };
        let err = parse(&yaml("shraed")).unwrap_err().to_string();
        assert_eq!(
            err,
            "unknown component `shraed` in common; did you mean `shared`?"
        );
        // Nothing is close enough: no guess.
        let err = parse(&yaml("payments")).unwrap_err().to_string();
        assert_eq!(err, "unknown component `payments` in common");
    }

    #[test]
    fn an_indexer_setting_must_be_one_the_indexer_takes() {
        let ok = parse(
            "version: 1\ncomponents:\n  a: { in: a }\nindexers: { go: { tags: [race, e2e_1] } }\n",
        );
        assert_eq!(ok.unwrap().indexers["go"]["tags"], ["race", "e2e_1"]);
        for (bad, says) in [
            (
                "{ go: { tags: [\"a b\"] } }",
                "indexers.go.tags: `a b` is not a build tag",
            ),
            (
                "{ go: { tag: [race] } }",
                "`tag` is no setting of the go indexer; one of: tags",
            ),
            (
                "{ gp: {} }",
                "indexers: `gp` is no language easement indexes",
            ),
            (
                "{ python: { tags: [x] } }",
                "the python indexer takes no setting",
            ),
        ] {
            let yaml = format!("version: 1\ncomponents:\n  a: {{ in: a }}\nindexers: {bad}\n");
            let err = format!("{:#}", parse(&yaml).unwrap_err());
            assert!(err.contains(says), "{bad}: {err}");
        }
    }

    #[test]
    fn a_part_cannot_be_isolated() {
        let err = parse(
            "version: 1
components:
  a: { in: 'crates/*/Cargo.toml', part: feature/fake, isolated: true }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("part"), "{err}");
    }

    #[test]
    fn isolated_needs_a_star_and_importers_must_exist() {
        let err = parse(
            "version: 1
components:
  a: { in: src/a, isolated: true }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("isolated"));
        let err = parse(
            "version: 1
components:
  a: { in: 'src/**', isolated: true }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("isolated"));
        let err = parse(
            "version: 1
components:
  a: { in: a, allow_importers: [ghost] }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn checks_placeholders() {
        let error = |component: &str, extra: &str| {
            let yaml = format!("version: 1\ncomponents:\n  a: {component}\n{extra}");
            format!("{:#}", parse(&yaml).unwrap_err())
        };
        assert!(error("{ in: 'src/x<f>/ui' }", "").contains("whole segment"));
        assert!(error("{ in: 'src/<F>/ui' }", "").contains("whole segment"));
        assert!(error("{ in: 'src/**/<f>' }", "").contains("after `**`"));
        assert!(error("{ in: 'src/<f>/<f>' }", "").contains("twice"));
        assert!(error("{ in: src/a }", "exclude: ['src/<f>']\n").contains("works only in `in`"));
        assert!(error("{ in: src/a, public: 'src/<f>/api' }", "").contains("works only in `in`"));
        assert!(parse("version: 1\ncomponents:\n  a: { in: 'src/<f>', isolated: true }\n").is_ok());
    }

    #[test]
    fn a_named_surface_lies_inside_its_component_and_names_admitted_importers() {
        let error = |billing: &str| {
            let yaml = format!(
                "version: 1\ncomponents:\n  jobs: {{ in: src/jobs }}\n  checkout: {{ in: src/checkout }}\n  billing: {billing}\n"
            );
            parse(&yaml).err().map(|e| format!("{e:#}"))
        };
        let surface =
            |admin: &str| format!("{{ in: src/billing, surfaces: {{ admin: {admin} }} }}");
        assert_eq!(
            error(&surface("{ in: src/billing/admin, importers: [jobs] }")),
            None
        );
        assert_eq!(
            error(&surface(
                "{ in: 'src/billing/*/admin', importers: [jobs, checkout] }"
            )),
            None
        );
        let err = |text: &str| error(text).unwrap_or_default();
        assert!(
            err(&surface("{ in: src/shop/admin, importers: [jobs] }")).contains(
                "components.billing.surfaces.admin: `src/shop/admin` is outside the component"
            )
        );
        assert!(
            err(&surface("{ in: src/billingx, importers: [jobs] }"))
                .contains("outside the component")
        );
        // A path the component gives up with `except` is not its own.
        assert!(
            err("{ in: src/billing, except: src/billing/legacy, surfaces: { old: { in: src/billing/legacy/admin, importers: [jobs] } } }")
                .contains("`src/billing/legacy/admin` is outside the component")
        );
        assert!(
            err(&surface("{ in: src/billing/admin, importers: [jbos] }"))
                .contains("unknown component `jbos` in components.billing.surfaces.admin.importers; did you mean `jobs`?")
        );
        assert!(err(&surface("{ in: src/billing/admin, importers: [] }")).contains("no importers"));
        assert!(err(&surface("{ in: [], importers: [jobs] }")).contains("no `in` patterns"));
        assert!(
            err(&surface("{ in: 'src/billing/<f>', importers: [jobs] }"))
                .contains("works only in `in`")
        );
        assert!(
            err("{ in: src/billing, allow_importers: [checkout], surfaces: { admin: { in: src/billing/admin, importers: [jobs] } } }")
                .contains("components.billing.surfaces.admin opens to `jobs`, which allow_importers does not admit")
        );
        assert_eq!(
            error(
                "{ in: src/billing, allow_importers: [checkout, jobs], surfaces: { admin: { in: src/billing/admin, importers: [jobs] } } }"
            ),
            None
        );
    }

    #[test]
    fn checks_std_macros() {
        let error = |rule: &str| {
            let yaml = format!("version: 1\ncomponents:\n  a: {{ in: a }}\nrules:\n  a: {rule}\n");
            format!("{:#}", parse(&yaml).unwrap_err())
        };
        assert!(error("{ may_use_external: [$stdlib] }").contains("the only `$` name is `$std`"));
        assert!(error("{ must_use_external: [$std] }").contains("works only in"));
        assert!(parse("version: 1\ncomponents:\n  a: { in: a }\nrules:\n  a: { forbid_external: [$std] }\n").is_ok());
    }

    #[test]
    fn an_example_names_one_target_and_a_rule_that_can_reject_it() {
        let with = |example: &str| {
            parse(&format!(
                "version: 1\ncomponents:\n  a: {{ in: a }}\nexamples:\n  - {example}\n"
            ))
        };
        with("{ from: a/x, to: a/y, expect: rejected, rule: privacy }").unwrap();
        with("{ from: a/x, external: lodash, expect: rejected, rule: external }").unwrap();
        for (example, error) in [
            (
                "{ from: a/x, expect: allowed }",
                "exactly one of `to` and `external`",
            ),
            (
                "{ from: a/x, to: a/y, external: lodash, expect: allowed }",
                "exactly one of `to` and `external`",
            ),
            (
                "{ from: a/x, to: a/y, expect: allowed, rule: privacy }",
                "expects allowed, so no rule `privacy` can reject it",
            ),
            (
                "{ from: a/x, to: a/y, expect: rejected, rule: external }",
                "only an `external` module can be rejected by rule `external`",
            ),
            (
                "{ from: a/x, external: lodash, expect: rejected, rule: dependency }",
                "rejected only by rule `external`, never `dependency`",
            ),
            (
                "{ from: a/x, to: a/y, expect: rejected, rule: cycle }",
                "unknown variant `cycle`",
            ),
            (
                "{ from: a/x, to: a/y, expect: maybe }",
                "unknown variant `maybe`",
            ),
        ] {
            let err = format!("{:#}", with(example).unwrap_err());
            assert!(err.contains(error), "{example}: {err}");
        }
    }

    #[test]
    fn rejects_contradictions() {
        let err = parse(
            "version: 1
components:
  a: { in: a }
  b: { in: b }
rules:
  a: { may_depend_on: [b], must_not_depend_on: [b] }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("both allows and forbids"));
        let err = parse(
            "version: 1
components:
  a: { in: a }
  b: { in: b }
rules:
  a: { may_depend_on: [b], must_not_reach: [b] }
",
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("both allows `b` and forbids reaching it")
        );
        let err = parse(
            "version: 1
components:
  a: { in: a }
rules:
  a: { must_not_reach: [a] }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("names `a` itself"));
        let err = parse(
            "version: 1
components:
  a: { in: a }
layers: [a, a]
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("twice"));
        let err = parse(
            "version: 1
components:
  a: { in: a }
rules:
  a: { max_instability: 1.5 }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("max_instability"));
    }

    #[test]
    fn rejects_unknown_component() {
        let err = parse(
            "version: 1
components:
  a: { in: a }
rules:
  a: { may_depend_on: [ghost] }
",
        )
        .unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    /// A config of the components `domain`, `usecase` and `web`, with `rest` after them.
    fn three(rest: &str) -> Result<Config> {
        parse(&format!(
            "version: 1
components:
  domain: {{ in: domain }}
  usecase: {{ in: usecase }}
  web: {{ in: web }}
{rest}"
        ))
    }

    #[test]
    fn a_top_level_forbidden_name_may_allow_components() {
        let config = three(
            "forbid_symbol:\n  - { name: tags.DeleteIn, allow: [usecase, web], because: a use case }\n  - tags.Rename\n",
        )
        .unwrap();
        assert!(config.forbid_symbol[0].allows("usecase"));
        assert!(!config.forbid_symbol[0].allows("domain"));
        assert_eq!(config.forbid_symbol[1].allow(), None);
    }

    #[test]
    fn allow_works_only_in_the_top_level_forbid_symbol() {
        let err = three(
            "rules:\n  web: { forbid_symbol: [{ name: tags.DeleteIn, allow: [usecase] }] }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "rules.web.forbid_symbol `tags.DeleteIn` has `allow`, which works only in the top-level forbid_symbol"
        );
    }

    #[test]
    fn allow_names_known_components() {
        let err =
            three("forbid_symbol: [{ name: tags.DeleteIn, allow: [usecsae] }]\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown component `usecsae` in forbid_symbol[0].allow; did you mean `usecase`?"
        );
    }

    #[test]
    fn an_empty_allow_is_refused() {
        let err = three("forbid_symbol: [{ name: tags.DeleteIn, allow: [] }]\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "forbid_symbol[0].allow lists no component; leave the key out instead"
        );
    }

    #[test]
    fn a_component_cannot_be_allowed_a_name_its_own_rule_forbids() {
        for (own, said) in [
            ("tags.DeleteIn", ""),
            // A shorter qualifier names the same symbol.
            (
                "example.com/app/tags.DeleteIn",
                " as `example.com/app/tags.DeleteIn`",
            ),
        ] {
            let err = three(&format!(
                "forbid_symbol: [{{ name: tags.DeleteIn, allow: [usecase] }}]\nrules:\n  usecase: {{ forbid_symbol: [{own}] }}\n"
            ))
            .unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "forbid_symbol[0].allow lets `usecase` use `tags.DeleteIn`, which rules.usecase.forbid_symbol forbids{said}"
                )
            );
        }
        // A name under another package is another name.
        assert!(three("forbid_symbol: [{ name: tags.DeleteIn, allow: [usecase] }]\nrules:\n  usecase: { forbid_symbol: [labels.DeleteIn] }\n").is_ok());
    }

    #[test]
    fn a_name_with_allow_is_listed_once_at_the_top_level() {
        for (list, second) in [
            (
                "[{ name: tags.DeleteIn, allow: [usecase] }, tags.DeleteIn]",
                "tags.DeleteIn",
            ),
            (
                "[{ name: tags.DeleteIn, allow: [web] }, example.com/app/tags.DeleteIn]",
                "example.com/app/tags.DeleteIn",
            ),
        ] {
            let err = three(&format!("forbid_symbol: {list}\n")).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "forbid_symbol[0] `tags.DeleteIn` and forbid_symbol[1] `{second}` can name the same symbol, and one of them has `allow`, so who may use it is ambiguous: list it once"
                ),
                "{list}"
            );
        }
        let err = three("forbid_symbol: [tags.DeleteIn, { name: example.com/app/tags.DeleteIn, allow: [web] }]\n").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("forbid_symbol[0] `tags.DeleteIn` and forbid_symbol[1]"),
            "{err}"
        );
        // Without `allow`, a name listed twice says the same twice.
        assert!(three("forbid_symbol: [tags.DeleteIn, tags.DeleteIn]\n").is_ok());
        // Other names, other packages.
        assert!(three("forbid_symbol: [{ name: tags.DeleteIn, allow: [web] }, labels.DeleteIn, xtags.DeleteIn]\n").is_ok());
    }
}
