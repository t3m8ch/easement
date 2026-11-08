---
status: accepted
---

# A dependency is a name the code writes, in every language

`may_depend_on` meant two things. In PHP, Java, C# and Ruby a member use (a method call, a constructor) made a dependency on its class, so a method called on a value of inferred type counted. In Rust, TypeScript, Python and Go only a module reference made one, so a re-exported name or an inferred member did not ([#104](https://github.com/mokevnin/easement/issues/104)). The scheme checks in the core loop that caused this are gone. A dependency is now defined once: a name a file writes that is defined elsewhere, meaning a module or a name at the top level of one, never a member reached through a value. Every other reference is a symbol use, which the symbol rules read and the dependency rules do not. We chose this because what an indexer resolves for a written name is reliable in every language, while what it infers for a member is not: scip-ruby reports a call only where Sorbet knows the receiver's type, and scip-python types partially. Counting members would give one policy a different completeness in each language.

## Considered options

- **Every symbol use is a dependency, members included** (ArchUnit's model). This was rejected because the verdict would depend on how much each indexer infers, and because inferred, inherited and promoted uses would count: a method called on a value another component returned, or inherited through a third component's subclass.
- **Keep the per-language readings and state them in the registry and the report.** This was rejected because it names the inconsistency instead of removing it, and it contradicts "a rule is defined once for every language".

## Consequences

- A re-exported name (Rust `pub use`, a TypeScript barrel, a Python `__init__` import) is a dependency on the component that defines it, because the index points the use at the original. The re-exporting component's lines that refer to the original are evidence of that dependency, so `--diff` reports the use when only the re-export changed, and the violation names the re-exporter ([#103](https://github.com/mokevnin/easement/issues/103)).
- An alias that is its own definition is followed to what it stands for ([#112](https://github.com/mokevnin/easement/issues/112)): writing it is a dependency on the alias's component and on the component of every internal top-level name its definition references, followed through chains of aliases. A use of it is also a symbol use of those names. An alias is only what the index marks as one: a `TypeAlias` definition with its range (Go `type Block = blob.Block`, Rust `pub type`, TypeScript `export type`), or a definition related to another name by `is_reference` (Ruby `Block = Blob::Block`). The index states nothing else, so a variable that holds another name (TypeScript `export const`, any Python assignment) is a value and hides it. Kotlin and Swift `typealias` are not followed either: their adapters resolve only imports. External names are not followed: an external module is judged at the import that names it.
- In Java, PHP, C# and Ruby a method called on a value whose type the file never writes is no longer a dependency. It is a symbol use, and whether symbol rules judge it in those languages is up to the registry.
- Some indexers record a written class name only as the class's constructor: scip-java for `new X()`, scip-dotnet for an attribute `[X]`. Reading the index, a constructor is read as its class. It is the only member treated this way. Ruby's `initialize` is not: its occurrence stands on `new`, and the class's own symbol is already beside it.
- The clauses about files judge the modules a file names, not the names it writes. A surface (`public`, a named surface) lets a name through when a public file re-exports it, wherever it is defined. A name a file reaches only through aliases is judged by the surfaces of its component, unless one of those aliases is defined in a file of that component open to the file, as a public file's re-export would be: otherwise an alias in a third component would open any file of it. A name that an excluded file defines (a generated `api.pb.go` beside `api.go`) is no dependency: only a module the code names fails as `excluded`.
- No compatibility mode: this ships as a breaking change.

## Fork correction: public barrels and alias expansion

A public barrel that re-exports an alias retains its facade when that alias is
expanded to private types of the same component. SCIP records the re-export as a
reference to the alias, not a new alias definition. The expansion therefore keeps
the re-export location as evidence when the consumer's occurrence names that
barrel and the barrel references the alias it imports. A barrel of a different
component cannot open the target's private surface. Ordinary private imports and
explicit symbol prohibitions remain checked.
