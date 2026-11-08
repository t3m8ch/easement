---
status: accepted
---

# Accept exact implicit augmentation relationships in the fork

This fork implements the project outcome of upstream issue #134 at the user's
explicit request, before upstream agreement. The default graph remains strict.
`accept_augmentations` selects consumer components, an exact external module
specifier and one exact project-relative declaration file, with a required reason.
Multiple declarations require separate entries. This deliberately accepts the
module-string relationship, including its contribution to transitive reachability;
it does not grant a dependency on the declaration's component.

SCIP TypeScript represents a module augmentation with a quoted module descriptor
under its declaration file. Symbol normalization recognizes that descriptor; the
reader marks a relationship only when the reference stands on a matching bare
module string. Dependency keys preserve this provenance separately from ordinary
imports, even when both target the same file or occur on the same line. Explicit
project symbols retain their own dependencies and symbol checks. Other languages
can supply equivalent normalized facts without changing the core policy.

Judgment filters the accepted relationships before dependency, cycle, diagram,
coupling and file/symbol reach checks. Symbol-flow edges carry the same provenance.
The scan and cached SCIP bytes remain policy independent: the base and head policy
can judge the same scan, and removing an acceptance restores the relationship even
on a cache hit. Manifest, symbol, required-import and unused-code evidence still
reads the complete physical scan. This is an architecture exception, not proof
that the accepted metadata has no runtime effects.

`stats.accepted_augmentations` lists each applied relationship, source line, module,
declaration and reason in JSON. Text checks state the same exception and the checks
that omit it. An unused acceptance changes no graph; the output lists applied
relationships, not a claim that every configured acceptance matched. Policy review
reports additions and removals of acceptances.

## Considered options

- An inline suppression leaves the reachability path intact.
- Allowing the target component permits ordinary imports too.
- Erasing augmentation information from the index loses useful evidence and alters
  default behavior for projects that want to check it.
- Removing edges by file pair conflates implicit metadata with direct imports of
  the same declaration file. Provenance must survive dependency deduplication.
