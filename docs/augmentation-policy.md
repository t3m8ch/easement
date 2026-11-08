# Accepting external-module augmentation relationships (fork)

The default checks include project module augmentations reported by SCIP. This
fork adds `accept_augmentations` for projects that deliberately accept selected
implicit relationships. It is currently recognized for TypeScript/JavaScript SCIP
module-string references to a quoted external-module augmentation descriptor.

For project-wide TanStack Router registration, use the same entry in each of the
project's independent component and environment policies:

```yaml
accept_augmentations:
  - module: '@tanstack/react-router'
    because: implicit global router registration typing is intentional
```

Only `module` and `because` are mandatory. The module specifier matches exactly:
`@tanstack/react-router` does not match another package or
`@tanstack/react-router/subpath`. No globs or package-prefix matching are supported.
The reason must be nonempty.

Omitting `consumers` accepts matching implicit relationships from all consumers,
including files outside named components and newly introduced components. Omitting
`declaration` accepts all augmentation declaration files for that exact module,
including newly generated or added files. For example, the entry above covers
`src/router.tsx`, `src/app/navigation.tsx` and `src/routeTree.gen.ts` without naming
them. It also accepts relationships between those augmentation files themselves.

Both selectors remain available independently to narrow the acceptance:

```yaml
accept_augmentations:
  - module: '@tanstack/react-router'
    consumers: [platform]
    declaration: src/router.tsx
    because: platform links deliberately use this router registration
```

A supplied `consumers` must be a nonempty list of known exact component names,
without implied descendant components. A supplied `declaration` must be one
nonempty exact project-relative file path, without globs or parent traversal.
Explicit null selectors, empty lists, empty strings and wrong value types fail;
only omission means all. Existing entries with both selectors keep their scope.
The declaration selector covers all augmentations of the selected module in that
file, rather than individual interfaces. A typo naming an existing but wrong
component or a declaration path with no matching relationship accepts nothing.

Only the implicit module-string relationships disappear from dependency and
reachability judgments, including `reach: symbols`. Their contribution to cycles,
diagrams and coupling counts is also omitted. Direct imports of the same file,
explicit project-symbol references and unrelated transitive paths remain checked.
`forbid_symbol`, manifest and unused-code checks keep their complete evidence.
No changes to the indexer, blanket type exclusions or component grants are needed.
The text report and JSON `stats.accepted_augmentations` list applied relationships
with their source location and reason. No entry appears for an acceptance that did
not match a relationship. Remove an entry to restore checking, including on cached
indexes; `--no-cache` can independently refresh index evidence.

## Reproducible installation for developers and CI

Use a full immutable commit SHA from the user's fork, in a checked-in setup script
or CI configuration. With Rust 1.99.1 and a C/C++ build toolchain installed:

```sh
cargo +1.99.1 install --locked --git https://github.com/t3m8ch/easement.git \
  --rev "$EASEMENT_REV" --root "$PWD/.tools/easement" easement
export PATH="$PWD/.tools/easement/bin:$PATH"
```

Set `EASEMENT_REV` to the reviewed full commit SHA, not the feature branch name.
Cache the installed directory in CI using the SHA, OS and architecture as keys.
The TypeScript checker also requires Node.js and unmodified
`@sourcegraph/scip-typescript@0.6.1` on PATH (install it with the application's
locked npm dependencies). A release archive from upstream does not contain this
fork feature. No fork release or tag is required for this source installation.

See [the fork decision](adr/0007-accept-exact-augmentation-relationships.md) for the
scope and tradeoffs.
