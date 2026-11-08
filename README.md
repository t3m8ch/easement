# easement

Architecture quality gate for polyglot codebases.

Declare the components of your system and which of them may depend on which. `easement` extracts the real dependencies from the code, builds the graph and fails the build when the code drifts from the declared architecture.

A Go backend and a TypeScript frontend share one policy model, one baseline and one report. Rules can protect public APIs, forbid dependencies and cycles, isolate features, and restrict symbols and type names. Coding agents can query the policy before writing an import and read violations with the reason behind each rule.

> **easement** — a legal right to cross someone else's land. In `easement`, every allowed dependency between components is an explicit easement; everything else is trespassing.

## Install

```sh
brew install mokevnin/tap/easement
```

Homebrew works on macOS and Linux. You can also use `cargo install easement`, the install script or release packages; see [installation options](https://github.com/mokevnin/easement/blob/main/docs/installation.md).

Most languages also need a SCIP indexer and the project's dependencies installed. From your project directory, run:

```sh
easement doctor
```

It reports missing indexers and how to install them. Follow the [indexer setup](https://github.com/mokevnin/easement/blob/main/docs/languages.md#indexers), then repeat until `doctor` exits 0.

## Quick start

Start from your project's current layout:

```sh
easement init
```

It writes `easement.yml` with components and allowances for the dependencies it finds. Review those allowances and remove the ones the architecture should forbid.

For example, handlers should reach storage through services. For a Go project with `internal/handlers`, `internal/services`, `internal/repository` and `internal/models`, the policy can be:

```yaml
version: 1

components:
  handlers:   { in: internal/handlers }
  services:   { in: internal/services }
  repository: { in: internal/repository }
  models:     { in: internal/models }

common: [models]

rules:
  handlers:
    may_depend_on: [services]
    because: handlers go through services, never straight to storage
  services: { may_depend_on: [repository] }
```

Run the check:

```sh
easement check
```

A handler importing the repository fails with the source location and the reason:

```text
internal/handlers/books.go:6: handlers -> repository is not allowed (written as internal/repository/books.go) [dependency]
  because: handlers go through services, never straight to storage
```

Change the handler to go through the service, then check again. `check` exits 0 when the code matches the policy, 1 on violations and 2 on an error, such as a broken config or a missing indexer. Additional exit codes are described in the [command reference](https://github.com/mokevnin/easement/blob/main/docs/commands.md#everyday-commands).

For an existing codebase, a [baseline](https://github.com/mokevnin/easement/blob/main/docs/adoption.md#baseline-adopting-on-an-existing-project) records today's debt so new violations fail while old ones can be fixed gradually. For a pull request, [diff mode](https://github.com/mokevnin/easement/blob/main/docs/adoption.md#diff-mode-judging-a-pull-request) judges what the change adds.

## Set up with a coding agent

Use the [setup prompt](https://github.com/mokevnin/easement/blob/main/docs/agent-setup.md#set-up-with-a-coding-agent) to have Claude Code, Codex, Cursor or another agent install the tools, propose a policy and wire the check into CI. Review the proposed rules: the agent sees the code as it is, and you decide which dependencies should be allowed.

Install the [agent skill](skills/easement/SKILL.md) for everyday work:

```sh
npx skills add mokevnin/easement
```

The skill teaches agents to query the policy before adding code, run the check afterwards and fix violations using `because`. Changes to the policy and accepting new baseline debt need a human decision. [Claude Code plugin installation](https://github.com/mokevnin/easement/blob/main/docs/agent-setup.md#agent-skill) is also available.

## Everyday commands

```sh
easement check                    # check the project
easement explain <file>           # who owns the file and what it may do
easement propose <file> <target>  # judge an import before writing it
easement why <a> <b>              # show the imports behind a dependency
easement impact --diff main      # find files depending on changed code
easement review --base main      # review what a policy change permits
easement suggest                  # find unused allowances and missing ones
```

Use `--format json` for structured results. Reports for CI include GitHub annotations, SARIF and JUnit. See the [command reference](https://github.com/mokevnin/easement/blob/main/docs/commands.md) and [CI integration](https://github.com/mokevnin/easement/blob/main/docs/adoption.md#in-ci); `easement <command> --help` lists the available flags.

## Languages

Go, TypeScript/JavaScript, Python, Rust, PHP, Java, C# and Ruby use [SCIP](https://scip-code.org/) indexers. Kotlin and Swift use tree-sitter adapters. Resolution and rule coverage differ by language; see [languages and indexers](https://github.com/mokevnin/easement/blob/main/docs/languages.md).

A green verdict covers the code the tool could read and judge. Reports explicitly list evidence gaps, deliberate narrowing such as `--diff`, and cache use. For CI, `easement check --require-evidence` also fails when evidence is incomplete. See [coverage and evidence gaps](https://github.com/mokevnin/easement/blob/main/docs/languages.md#indexers).

## Documentation

Start with the [documentation index](https://github.com/mokevnin/easement/blob/main/docs/README.md), or go directly to:

| Task | Guide |
| --- | --- |
| Install easement | [Installation](https://github.com/mokevnin/easement/blob/main/docs/installation.md) |
| Set up with an agent | [Setup prompt and skill](https://github.com/mokevnin/easement/blob/main/docs/agent-setup.md) |
| Define components and rules | [Configuration reference](https://github.com/mokevnin/easement/blob/main/docs/configuration.md) |
| Query and check the architecture | [Command reference](https://github.com/mokevnin/easement/blob/main/docs/commands.md) |
| Adopt in an existing project or CI | [Baseline, diff, CI and monorepos](https://github.com/mokevnin/easement/blob/main/docs/adoption.md) |
| Set up indexers or understand analysis limits | [Languages, indexers and cache](https://github.com/mokevnin/easement/blob/main/docs/languages.md) |
| Compare architecture tools | [Comparison and inspirations](https://github.com/mokevnin/easement/blob/main/docs/comparison.md) |

## Development

Tools are pinned in `mise.toml`: `mise install`, which also installs the git hooks, `mise run setup` for the npm dependencies, then `mise run check` (every lint, the same set CI runs) and `cargo test`. Commits follow Conventional Commits; releases are cut by merging the release-please pull request. Before proposing a new language, rule or config key, read [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT

This fork supports opt-in [external-module augmentation acceptances](docs/augmentation-policy.md)
for narrowly selected metadata relationships, while keeping ordinary imports checked.
