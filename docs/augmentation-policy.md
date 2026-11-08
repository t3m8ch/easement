# Accepting external-module augmentation relationships (fork)

The default checks include project module augmentations reported by SCIP. This
fork adds `accept_augmentations` for projects that deliberately accept selected
implicit relationships. It is currently recognized for TypeScript/JavaScript SCIP
module-string references to a quoted external-module augmentation descriptor.

For TanStack Router registration, add this to the component policy:

```yaml
accept_augmentations:
  - consumers: [platform]
    module: '@tanstack/react-router'
    declaration: src/router.tsx
    because: platform links deliberately use the application router registration
  - consumers: [platform]
    module: '@tanstack/react-router'
    declaration: src/app/navigation.tsx
    because: platform links deliberately use static route metadata
  - consumers: [platform]
    module: '@tanstack/react-router'
    declaration: src/routeTree.gen.ts
    because: platform links deliberately use generated route typing
```

Use actual component names from each policy. For a separate environment policy,
repeat the entries with `consumers: [ui]` if that policy assigns those consumers to
`ui`. Paths and module specifiers match exactly, without globs, package-prefix
matching or implied descendant components. A subpath import needs its own entry.
The declaration path selects the augmentation module in that file, not individual
interfaces within it. Multiple augmentations of the same module in one file are
accepted together.

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
