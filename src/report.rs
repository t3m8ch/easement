use anstyle::{AnsiColor, Style};
use quick_junit::{NonSuccessKind, TestCase, TestCaseStatus, TestSuite};
use serde::Serialize;
use serde_sarif::sarif::{self, ResultLevel};
use std::collections::BTreeMap;

use crate::baseline::{Severity, Verdict};
use crate::check::{self, Key, Place, RuleId, Stats, Unjudged, Violation};
use crate::components::AcceptedGap;
use crate::diff::{Changes, Diff, InFull};

#[derive(Serialize)]
struct JsonReport<'a> {
    passed: bool,
    violations: &'a [Violation],
    /// Violations of the rules in `warn`, which do not fail the check.
    warnings: &'a [Violation],
    /// Who to ask about a violation, by the component in its `from`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    owners: &'a BTreeMap<String, String>,
    stale: &'a [Key],
    legalized: usize,
    #[serde(flatten)]
    coverage: &'a Coverage,
    /// Set when indexes came from the cache: what their key leaves out and how
    /// to index again.
    #[serde(skip_serializing_if = "Option::is_none")]
    cached: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff: Option<DiffInfo<'a>>,
    stats: &'a Stats,
}

#[derive(Serialize)]
struct DiffInfo<'a> {
    base: &'a str,
    merge_base: &'a str,
    /// The policy inputs that differ from the merge-base: what the change to
    /// the policy causes over the same code is judged too.
    policy_changed: &'a [String],
    /// What was judged in full instead of by the change, with why; every rule
    /// when `keys` is absent.
    in_full: &'a [InFull],
}

/// How much of the project an answer covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Full,
    /// It rests on code no indexer read, or judges only a part by design.
    Partial,
}

/// What an answer leaves out: the part it was asked to judge, the code it could
/// not read or judge, and the gaps the policy accepts. A cache hit is none of them.
#[derive(Debug, Default, Serialize)]
pub struct Coverage {
    /// What the run was asked to judge, when not the whole project.
    pub narrowed: Vec<Narrowing>,
    /// Code or facts the run could not read or judge: a pass says nothing about them.
    pub evidence_gaps: Vec<EvidenceGap>,
    /// Files no indexer read that `uncovered_ok` accepts, by the entry that accepts them.
    pub accepted_gaps: Vec<AcceptedGap>,
}

/// A part of the project or of a change that a narrowed verdict judges alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Narrowing {
    pub kind: NarrowingKind,
    /// The base of `--diff`, the component of `--component`, the path of `--scope`.
    pub limited_to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrowingKind {
    Diff,
    Component,
    Scope,
    /// `propose --no-graph`: the decisions alone, not what they do to the graph.
    NoGraph,
}

/// Code or facts that a check could not read or judge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceGap {
    pub kind: GapKind,
    /// The files the gap is in, when they are known one by one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The extensions of the files the gap is in, when it holds for a whole language.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// How many files, when they are known one by one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// The places in those files the gap is at, when they are known one by one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
    pub message: String,
    /// How to close the gap.
    pub close: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    /// Source files of an indexed language that no indexer covered.
    Uncovered,
    /// Files whose types `defines` cannot see.
    NamingUnseen,
    /// Files whose standard library `$std` cannot tell.
    StdUnknown,
    /// Imports that `manifests` could not judge.
    Manifests,
    /// Test code that could not be told from the rest of its file.
    Parts,
    /// Manifests, or dependencies of one, that `packages` could not judge.
    Packages,
    /// Imports a file writes that the indexer did not resolve, so their
    /// dependencies are not in the graph.
    UnresolvedImports,
    /// Uses of names that `forbid_symbol` could not judge.
    Symbols,
    /// Files that `unused` took as used without judging them.
    Unused,
    /// Files a change deletes: what imported them is not in the code any more.
    Deleted,
}

impl GapKind {
    /// Whether the gap leaves dependencies out of the graph, so that an answer
    /// about it (`impact`, `why`) may be short; the rest leave a rule unjudged.
    pub fn hides_dependencies(self) -> bool {
        match self {
            GapKind::Uncovered
            | GapKind::Parts
            | GapKind::Packages
            | GapKind::UnresolvedImports
            | GapKind::Deleted => true,
            GapKind::NamingUnseen
            | GapKind::StdUnknown
            | GapKind::Manifests
            | GapKind::Symbols
            | GapKind::Unused => false,
        }
    }
}

/// The parts that `--diff`, `--component` and `--scope` narrow a check to.
pub fn narrowed(
    changes: Option<&Changes>,
    components: &[String],
    paths: &[String],
) -> Vec<Narrowing> {
    let item = |kind, limited_to: &String| Narrowing {
        kind,
        limited_to: limited_to.clone(),
    };
    changes
        .map(|c| item(NarrowingKind::Diff, &c.base))
        .into_iter()
        .chain(components.iter().map(|c| item(NarrowingKind::Component, c)))
        .chain(paths.iter().map(|p| item(NarrowingKind::Scope, p)))
        .collect()
}

/// The narrowing by `--component` and `--scope`, as `component web` and
/// `path src/api`: what `stats.limited_to` holds.
pub fn limited_to(narrowed: &[Narrowing]) -> Vec<String> {
    narrowed
        .iter()
        .filter_map(|n| match n.kind {
            NarrowingKind::Diff | NarrowingKind::NoGraph => None,
            NarrowingKind::Component => Some(format!("component {}", n.limited_to)),
            NarrowingKind::Scope => Some(format!("path {}", n.limited_to)),
        })
        .collect()
}

impl EvidenceGap {
    /// The gap of what a rule could not judge, with how to close it.
    pub fn of(kind: GapKind, unjudged: &Unjudged, close: &str) -> Self {
        EvidenceGap {
            kind,
            files: unjudged.files.clone(),
            extensions: unjudged.extensions.clone(),
            count: (!unjudged.files.is_empty()).then_some(unjudged.files.len()),
            places: unjudged.places.clone(),
            message: unjudged.reason.clone(),
            close: close.to_string(),
        }
    }
}

pub const CLOSE_UNCOVERED: &str =
    "add them to a project the indexer reads, `exclude` them, or accept them in `uncovered_ok`";

/// Code a check could not read or judge, in the order the reasons were always given.
pub fn evidence_gaps(stats: &Stats) -> Vec<EvidenceGap> {
    evidence_gaps_unread(stats, &stats.uncovered)
}

/// The evidence gaps of a run that leave dependencies out of its graph, for an
/// answer about the graph (`impact`, `why`). `unread` is every file no indexer
/// read, `uncovered_ok` or not: accepting a file settles the verdict of `check`,
/// but nobody saw its imports.
pub fn graph_gaps(stats: &Stats, unread: &[String]) -> Vec<EvidenceGap> {
    evidence_gaps_unread(stats, unread)
        .into_iter()
        .filter(|gap| gap.kind.hides_dependencies())
        .collect()
}

/// The evidence gaps of a run, with `unread` as the files no indexer read.
fn evidence_gaps_unread(stats: &Stats, unread: &[String]) -> Vec<EvidenceGap> {
    let mut gaps = Vec::new();
    let mut add = |kind, unjudged: &[Unjudged], close: &str| {
        gaps.extend(unjudged.iter().map(|u| EvidenceGap::of(kind, u, close)));
    };
    add(
        GapKind::Unused,
        &stats.unused_partial,
        "the files it took as used may be dead: review them by hand, or have an indexer read them",
    );
    add(
        GapKind::Manifests,
        &stats.manifests_partial,
        "fix or add the manifest the message names, or review those dependencies by hand",
    );
    add(
        GapKind::Parts,
        &stats.parts_partial,
        "fix what the message names, or review that code by hand",
    );
    add(
        GapKind::Packages,
        &stats.packages_partial,
        "fix the manifest the message names, or review those dependencies by hand",
    );
    add(
        GapKind::UnresolvedImports,
        &stats.imports_partial,
        "build the code and fix what the compiler reports, then check again",
    );
    add(
        GapKind::Symbols,
        &stats.symbols_partial,
        "do what the message says, or review those uses by hand",
    );
    if !stats.naming_unseen.is_empty() {
        add(
            GapKind::NamingUnseen,
            &[Unjudged::new(format!(
                "`defines` cannot see the types of .{} files, which are read without an index, so their names were not checked",
                stats.naming_unseen.join(", .")
            ))
            .in_extensions(stats.naming_unseen.iter().cloned())],
            "no indexer reads these files, so review the names of their types by hand",
        );
    }
    if !stats.std_unknown.is_empty() {
        add(
            GapKind::StdUnknown,
            &[Unjudged::new(format!(
                "`$std` cannot tell the standard library of .{} files, so their dependencies on it count as other modules; name those modules instead",
                stats.std_unknown.join(", .")
            ))
            .in_extensions(stats.std_unknown.iter().cloned())],
            "name the standard modules the rule means instead of `$std`",
        );
    }
    if !unread.is_empty() {
        add(
            GapKind::Uncovered,
            &[Unjudged::new(format!(
                "{} source files were not covered by an indexer, so the dependencies in or on them were not checked",
                unread.len()
            ))
            .in_files(unread.iter().cloned())],
            CLOSE_UNCOVERED,
        );
    }
    gaps
}

impl Coverage {
    /// What a check over `stats` leaves out, when it was narrowed to `narrowed`.
    pub fn of_check(
        stats: &Stats,
        narrowed: Vec<Narrowing>,
        accepted_gaps: Vec<AcceptedGap>,
    ) -> Self {
        Coverage {
            narrowed,
            evidence_gaps: evidence_gaps(stats),
            accepted_gaps,
        }
    }

    /// `partial` when the answer is narrowed or has an evidence gap; an
    /// accepted gap alone leaves it `full`.
    pub fn confidence(&self) -> Confidence {
        if self.narrowed.is_empty() && self.evidence_gaps.is_empty() {
            Confidence::Full
        } else {
            Confidence::Partial
        }
    }

    /// One sentence for `--diff`, and one for `--component` and `--scope` together.
    fn narrowed_reasons(&self) -> Vec<String> {
        let mut reasons: Vec<String> = self
            .narrowed
            .iter()
            .filter(|n| n.kind == NarrowingKind::Diff)
            .map(|n| {
                format!(
                    "only violations on lines changed since {}, or caused by the change to the policy, were judged, the rest of the project was not",
                    n.limited_to
                )
            })
            .collect();
        if self
            .narrowed
            .iter()
            .any(|n| n.kind == NarrowingKind::NoGraph)
        {
            reasons.push(
                "only the dependency decision of each proposal was judged, not the cycles, reach chains, limits and diagram arrows they cause (`--no-graph`)".to_string(),
            );
        }
        let limited = limited_to(&self.narrowed);
        if !limited.is_empty() {
            reasons.push(format!(
                "the report is limited to {}, the rest of the project was not judged",
                limited.join(", ")
            ));
        }
        reasons
    }
}

/// What a diff run judges, as the lines that open its text report.
fn diff_header(diff: &Diff) -> String {
    let Changes {
        base, merge_base, ..
    } = &diff.changes;
    let short = &merge_base[..merge_base.len().min(12)];
    let attribution = &diff.attribution;
    let mut out = if attribution.whole() {
        format!(
            "Checking the whole project, not only the lines changed since {base} (merge-base {short}):\n"
        )
    } else if attribution.policy_changed.is_empty() {
        format!("Checking lines changed since {base} (merge-base {short})\n")
    } else {
        format!(
            "Checking lines changed since {base} (merge-base {short}), and what the change to the policy causes\n"
        )
    };
    for part in &attribution.in_full {
        if part.keys.is_empty() {
            out.push_str(&format!("  {}\n", part.reason));
        } else {
            out.push_str(&format!("Judged in full: {}\n", part.reason));
        }
    }
    out
}

/// What a verdict over cached indexes rests on. Not a reason for `partial`: the
/// key holds what decides an index, but not dependencies outside the project.
pub fn cache_note(stats: &Stats) -> Option<String> {
    cache_note_of(stats.from_cache)
}

/// What a result resting on `from_cache` cached indexes rests on, if any.
pub fn cache_note_of(from_cache: usize) -> Option<String> {
    (from_cache > 0).then(|| {
        format!(
            "{from_cache} index(es) came from the cache, keyed on the project files, the indexer and its environment but not on dependencies outside the project; if those changed without a lock file, run with `--no-cache` to index again"
        )
    })
}

/// What a run of the check says: its verdict, what it saw, what it leaves out
/// and, in diff mode, the change it judged.
#[derive(Clone, Copy)]
pub struct Report<'a> {
    pub verdict: &'a Verdict,
    pub stats: &'a Stats,
    pub coverage: &'a Coverage,
    pub diff: Option<&'a Diff>,
    /// `--require-evidence`: an evidence gap fails a run without violations.
    pub require_evidence: bool,
}

/// How many files a text list names before it counts the rest.
const FILES_SHOWN: usize = 20;

const BOLD: Style = Style::new().bold();
const DIM: Style = Style::new().dimmed();
const RED: Style = AnsiColor::Red.on_default().bold();
const GREEN: Style = AnsiColor::Green.on_default().bold();
const YELLOW: Style = AnsiColor::Yellow.on_default();

/// `text` wrapped in the escape codes of `style`, or as it is when `color` is off.
fn paint(color: bool, style: Style, text: &str) -> String {
    if color {
        format!("{style}{text}{style:#}")
    } else {
        text.to_string()
    }
}

/// The first files or places of a list, one per line under `indent`, and how many more.
pub(crate) fn item_lines<T: std::fmt::Display>(items: &[T], indent: &str) -> String {
    check::name_first(
        items,
        FILES_SHOWN,
        "",
        |item| format!("{indent}{item}\n"),
        |more| format!("{indent}… and {more} more (all of them in `--format json`)\n"),
    )
}

/// The evidence gaps, each with how to close it and its files, then the
/// accepted gaps, each with its reason and files: every command says them so.
/// `silent` says what the report cannot vouch for (`a pass says nothing about this
/// code`). Under `--require-evidence` the gaps fail the run, and their heading says so.
pub fn gap_lines(
    evidence_gaps: &[EvidenceGap],
    accepted_gaps: &[AcceptedGap],
    silent: &str,
    required: bool,
    color: bool,
) -> String {
    let mut out = String::new();
    if !evidence_gaps.is_empty() {
        out.push_str(&if required {
            paint(
                color,
                RED,
                "Evidence required (`--require-evidence`), but the check could not read or judge this code:",
            )
        } else {
            paint(color, YELLOW, &format!("Evidence gaps ({silent}):"))
        });
        out.push('\n');
        for gap in evidence_gaps {
            out.push_str(&format!("  {}\n", paint(color, YELLOW, &gap.message)));
            out.push_str(&paint(color, DIM, &format!("    to close: {}", gap.close)));
            out.push('\n');
            // The places say the files too, each at its line.
            out.push_str(&if gap.places.is_empty() {
                item_lines(&gap.files, "    ")
            } else {
                item_lines(&gap.places, "    ")
            });
        }
    }
    if !accepted_gaps.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(
            "Accepted gaps (`uncovered_ok`: no indexer read them, so their dependencies were not checked):\n",
        );
        for gap in accepted_gaps {
            out.push_str(&format!("  {}", gap.uncovered_ok.join(", ")));
            if let Some(because) = &gap.because {
                out.push_str(&paint(color, DIM, &format!(" because: {because}")));
            }
            out.push('\n');
            out.push_str(&item_lines(&gap.files, "    "));
        }
    }
    out
}

/// `file:line: message [rule]`, and the reason under it when there is one.
pub(crate) fn violation_lines(
    v: &Violation,
    owner: Option<&str>,
    tag: Style,
    color: bool,
) -> String {
    let mut out = format!(
        "{}: {} {}\n",
        paint(color, BOLD, &format!("{}:{}", v.file, v.line)),
        v.message,
        paint(color, tag, &format!("[{}]", v.rule.as_str()))
    );
    if let Some(because) = &v.because {
        out.push_str(&paint(color, DIM, &format!("  because: {because}")));
        out.push('\n');
    }
    if let Some(owner) = owner {
        out.push_str(&paint(color, DIM, &format!("  owner: {owner}")));
        out.push('\n');
    }
    out
}

impl Report<'_> {
    /// Whether `--require-evidence` fails the run: it has an evidence gap.
    /// Violations and stale entries fail it first.
    pub fn evidence_required(self) -> bool {
        self.require_evidence && !self.coverage.evidence_gaps.is_empty()
    }

    /// Whether the run passes: no violations, no stale entries, and the
    /// evidence `--require-evidence` asks for.
    pub fn passed(self) -> bool {
        self.verdict.passed() && !self.evidence_required()
    }

    /// The report as data.
    pub fn json(self) -> String {
        let Report {
            verdict,
            stats,
            coverage,
            diff,
            require_evidence: _,
        } = self;
        let report = JsonReport {
            passed: self.passed(),
            violations: &verdict.new,
            warnings: &verdict.warnings,
            owners: &verdict.owners,
            stale: &verdict.stale,
            legalized: verdict.legalized,
            coverage,
            cached: cache_note(stats),
            diff: diff.map(|d| DiffInfo {
                base: &d.changes.base,
                merge_base: &d.changes.merge_base,
                policy_changed: &d.attribution.policy_changed,
                in_full: &d.attribution.in_full,
            }),
            stats,
        };
        serde_json::to_string_pretty(&report).expect("report serializes")
    }

    /// The plain text report.
    pub fn text(self) -> String {
        self.render(false)
    }

    /// The text report with ANSI styles; the caller strips them where the output
    /// is not a terminal.
    pub fn colored(self) -> String {
        self.render(true)
    }

    fn render(self, color: bool) -> String {
        let Report {
            verdict,
            stats,
            coverage,
            diff,
            require_evidence: _,
        } = self;
        let mut out = String::new();
        if let Some(d) = diff {
            out.push_str(&diff_header(d));
            out.push('\n');
        }
        for v in &verdict.new {
            out.push_str(&violation_lines(
                v,
                verdict.owner_of(v).as_deref(),
                RED,
                color,
            ));
        }
        if !verdict.warnings.is_empty() {
            let mut body = paint(
                color,
                YELLOW,
                "Warnings (rules in `warn`; do not add more):",
            );
            body.push('\n');
            for v in &verdict.warnings {
                body.push_str(&violation_lines(
                    v,
                    verdict.owner_of(v).as_deref(),
                    YELLOW,
                    color,
                ));
            }
            section(&mut out, &body);
        }
        if !verdict.stale.is_empty() {
            let mut body = paint(
                color,
                YELLOW,
                "Stale baseline entries (fixed, run `easement baseline` to shrink it):",
            );
            body.push('\n');
            for k in &verdict.stale {
                let what = stale_description(k);
                body.push_str(&format!("  [{}] {what}\n", k.rule.as_str()));
            }
            section(&mut out, &body);
        }
        if !stats.new_dependencies.is_empty() {
            let mut body = String::from("New dependencies between components in this change:\n");
            for d in &stats.new_dependencies {
                body.push_str(&format!("  {d}\n"));
            }
            section(&mut out, &body);
        }
        for accepted in &stats.accepted_augmentations {
            section(
                &mut out,
                &format!(
                    "Accepted augmentation: {}:{} -> {} for {}: {}; omitted from dependency and reach checks\n",
                    accepted.file,
                    accepted.line,
                    accepted.declaration,
                    accepted.module,
                    accepted.because
                ),
            );
        }
        let gaps = gap_lines(
            &coverage.evidence_gaps,
            &coverage.accepted_gaps,
            "a pass says nothing about this code",
            self.evidence_required(),
            color,
        );
        if !gaps.is_empty() {
            section(&mut out, &gaps);
        }
        if let Some(note) = cache_note(stats) {
            section(&mut out, &format!("{note}\n"));
        }
        section(&mut out, &self.summary(color));
        out.push('\n');
        out
    }

    /// The last line of the text report: the verdict and its counts.
    fn summary(self, color: bool) -> String {
        let Report { verdict, stats, .. } = self;
        let status = if self.passed() {
            paint(color, GREEN, "passed")
        } else {
            paint(color, RED, "failed")
        };
        let warnings = if verdict.warnings.is_empty() {
            String::new()
        } else {
            format!(", {} warning(s)", verdict.warnings.len())
        };
        let mut out = format!(
            "easement {status}: {} violation(s), {} stale, {} legalized{warnings}; {} files, {} internal deps",
            verdict.new.len(),
            verdict.stale.len(),
            verdict.legalized,
            stats.files,
            stats.internal_deps,
        );
        if !stats.limited_to.is_empty() {
            out.push_str(&format!(", limited to {}", stats.limited_to.join(", ")));
        }
        if stats.suppressed > 0 {
            out.push_str(&format!(", {} suppressed", stats.suppressed));
        }
        if !stats.uncovered.is_empty() {
            out.push_str(&format!(
                ", {} files not covered by an indexer",
                stats.uncovered.len()
            ));
        }
        if stats.unmapped_files > 0 {
            out.push_str(&format!(
                ", {} files outside components",
                stats.unmapped_files
            ));
        }
        out
    }
}

/// Appends a section of the text report, a blank line before it unless it is the first.
fn section(out: &mut String, body: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(body);
}

fn stale_description(k: &Key) -> String {
    match k.rule {
        RuleId::Diagram => format!("{} -> {} is now in the diagram or gone", k.from, k.to),
        _ => key_description(k),
    }
}

/// What the violation a baseline entry legalizes is, in one line.
pub fn key_description(k: &Key) -> String {
    match k.rule {
        RuleId::Cycle => format!("cycle among {}", k.from),
        RuleId::Metric => format!("{} exceeds {}", k.from, k.target),
        RuleId::Reach => format!("{} reaches {}", k.from, k.to),
        RuleId::Diagram => format!("{} -> {} is not in the diagram", k.from, k.to),
        RuleId::External => format!("{}: {} -> external {}", k.file, k.from, k.target),
        RuleId::Dependency | RuleId::Privacy => {
            format!("{}: {} -> {} ({})", k.file, k.from, k.to, k.target)
        }
        _ => format!("{}: {}", k.file, k.target),
    }
}

fn stale_message(k: &Key) -> String {
    format!(
        "stale baseline entry, fixed debt: {} (run `easement baseline` to shrink it)",
        stale_description(k)
    )
}

impl Report<'_> {
    /// What a run says of itself beside its results, in the order the text gives
    /// it: the narrowing, the evidence gaps (errors under `--require-evidence`),
    /// and the cache. Each is (level, text, files).
    fn notes(&self) -> Vec<(ResultLevel, String, &[String])> {
        let mut notes: Vec<(ResultLevel, String, &[String])> = self
            .coverage
            .narrowed_reasons()
            .into_iter()
            .map(|reason| (ResultLevel::Note, reason, &[][..]))
            .collect();
        let level = gap_level(self.require_evidence);
        for gap in &self.coverage.evidence_gaps {
            let mut text = format!("{}; to close: {}", gap.message, gap.close);
            if !gap.files.is_empty() {
                text.push_str(&format!("; files: {}", gap.files.join(", ")));
            }
            notes.push((level, text, &gap.files));
        }
        if let Some(note) = cache_note(self.stats) {
            notes.push((ResultLevel::Note, note, &[][..]));
        }
        notes
    }

    /// SARIF 2.1.0, the format GitHub Code Scanning ingests: the violations as
    /// results, and what the run leaves out as notifications of its invocation.
    pub fn sarif(&self) -> String {
        let notifications: Vec<_> = self
            .notes()
            .into_iter()
            .map(|(level, text, files)| {
                sarif::Notification::builder()
                    .level(serde_json::to_value(level).expect("a level serializes"))
                    .message(sarif_message(text))
                    .locations(
                        files
                            .iter()
                            .map(|file| sarif_location(file, None))
                            .collect::<Vec<_>>(),
                    )
                    .build()
            })
            .collect();
        sarif(self.verdict, notifications)
    }

    /// JUnit XML: one failed test case per violation, and under `--require-evidence`
    /// per evidence gap; what the run leaves out also in `<system-out>`.
    pub fn junit(&self) -> String {
        let gaps: Vec<(String, String)> = if self.require_evidence {
            self.coverage
                .evidence_gaps
                .iter()
                .map(|gap| (gap.message.clone(), gap.close.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let out: Vec<String> = self.notes().into_iter().map(|(_, text, _)| text).collect();
        junit(self.verdict, &gaps, &out)
    }
}

/// SARIF 2.1.0 of a verdict, with what the run says of itself.
fn sarif(verdict: &Verdict, notifications: Vec<sarif::Notification>) -> String {
    let mut ids: Vec<&str> = verdict
        .new
        .iter()
        .chain(&verdict.warnings)
        .map(|v| v.rule.as_str())
        .collect();
    if !verdict.stale.is_empty() {
        ids.push(STALE);
    }
    ids.sort_unstable();
    ids.dedup();
    let rules: Vec<_> = ids
        .iter()
        .map(|id| {
            sarif::ReportingDescriptor::builder()
                .id(*id)
                .short_description(
                    sarif::MultiformatMessageString::builder()
                        .text(format!("easement {id}"))
                        .build(),
                )
                .build()
        })
        .collect();
    let mut results: Vec<_> = verdict
        .reported()
        .map(|(severity, v)| {
            sarif::Result::builder()
                .rule_id(v.rule.as_str())
                .level(ResultLevel::from(severity))
                .message(sarif_message(message_with_reason(v)))
                .locations(vec![sarif_location(&v.file, Some(v.line))])
                .build()
        })
        .collect();
    results.extend(verdict.stale.iter().map(|k| {
        sarif::Result::builder()
            .rule_id(STALE)
            .level(ResultLevel::Error)
            .message(sarif_message(stale_message(k)))
            .build()
    }));
    let driver = sarif::ToolComponent::builder()
        .name("easement")
        .version(env!("CARGO_PKG_VERSION"))
        .information_uri(env!("CARGO_PKG_HOMEPAGE"))
        .rules(rules)
        .build();
    let invocation = sarif::Invocation::builder()
        .execution_successful(true)
        .tool_execution_notifications(notifications)
        .build();
    let run = sarif::Run::builder()
        .tool(sarif::Tool::builder().driver(driver).build())
        .results(results)
        .invocations(vec![invocation])
        .build();
    let report = sarif::Sarif::builder()
        .schema("https://json.schemastore.org/sarif-2.1.0.json")
        .version("2.1.0")
        .runs(vec![run])
        .build();
    serde_json::to_string_pretty(&report).expect("report serializes")
}

fn sarif_message(text: String) -> sarif::Message {
    sarif::Message::builder().text(text).build()
}

/// Where a result or a notification is: the file, and the line when there is one.
fn sarif_location(file: &str, line: Option<usize>) -> sarif::Location {
    let artifact = sarif::ArtifactLocation::builder().uri(file).build();
    let physical = match line {
        Some(line) => sarif::PhysicalLocation::builder()
            .artifact_location(artifact)
            .region(
                sarif::Region::builder()
                    .start_line(i64::try_from(line).expect("a line fits in i64"))
                    .build(),
            )
            .build(),
        None => sarif::PhysicalLocation::builder()
            .artifact_location(artifact)
            .build(),
    };
    sarif::Location::builder()
        .physical_location(physical)
        .build()
}

impl From<Severity> for ResultLevel {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Error => ResultLevel::Error,
            Severity::Warning => ResultLevel::Warning,
        }
    }
}

/// The level of an evidence gap: an error when the run requires evidence.
fn gap_level(require_evidence: bool) -> ResultLevel {
    if require_evidence {
        ResultLevel::Error
    } else {
        ResultLevel::Warning
    }
}

/// The GitHub workflow command that annotates at `level`.
fn workflow_command(level: ResultLevel) -> &'static str {
    match level {
        ResultLevel::Error => "error",
        ResultLevel::Warning => "warning",
        ResultLevel::Note | ResultLevel::None => "notice",
    }
}

/// The message of a violation, with its reason in parentheses when there is one.
fn message_with_reason(v: &Violation) -> String {
    match &v.because {
        Some(because) => format!("{} (because: {because})", v.message),
        None => v.message.clone(),
    }
}

fn escape_command(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_property(text: &str) -> String {
    escape_command(text).replace(':', "%3A").replace(',', "%2C")
}

impl Report<'_> {
    /// GitHub Actions workflow commands: annotations on the changed files.
    pub fn github(self) -> String {
        let Report {
            verdict,
            stats,
            coverage,
            diff,
            require_evidence: _,
        } = self;
        let mut out = String::new();
        for (severity, v) in verdict.reported() {
            let mut text = message_with_reason(v);
            if let Some(owner) = verdict.owner_of(v) {
                text.push_str(&format!(" (owner: {owner})"));
            }
            out.push_str(&format!(
                "::{} file={},line={},title={}::{}\n",
                workflow_command(severity.into()),
                escape_property(&v.file),
                v.line,
                escape_property(&format!("easement {}", v.rule.as_str())),
                escape_command(&text)
            ));
        }
        for k in &verdict.stale {
            out.push_str(&format!(
                "::error title={}::{}\n",
                escape_property(&format!("easement {STALE}")),
                escape_command(&stale_message(k))
            ));
        }
        if !stats.new_dependencies.is_empty() {
            out.push_str(&format!(
                "::notice title={}::{}\n",
                escape_property("easement graph"),
                escape_command(&format!(
                    "New component dependencies in this change: {}",
                    stats.new_dependencies.join(", ")
                ))
            ));
        }
        let narrowed = coverage
            .narrowed_reasons()
            .into_iter()
            .map(|reason| (ResultLevel::Note, reason));
        for part in diff.iter().flat_map(|d| &d.attribution.in_full) {
            out.push_str(&format!(
                "::notice title={}::{}\n",
                escape_property("easement diff"),
                escape_command(&format!("Judged in full: {}", part.reason))
            ));
        }
        let level = gap_level(self.evidence_required());
        let gaps = coverage
            .evidence_gaps
            .iter()
            .map(|gap| (level, gap.message.clone()));
        for (level, reason) in narrowed.chain(gaps) {
            out.push_str(&format!(
                "::{} title={}::{}\n",
                workflow_command(level),
                escape_property("easement partial"),
                escape_command(&reason)
            ));
        }
        if let Some(note) = cache_note(stats) {
            out.push_str(&format!(
                "::notice title={}::{}\n",
                escape_property("easement cache"),
                escape_command(&note)
            ));
        }
        out.push_str(&self.summary(false));
        out.push('\n');
        out
    }
}

/// A failed test case of `kind`: its message, and the text of the failure.
fn failed(kind: &str, name: String, message: String, body: String) -> TestCase {
    let mut status = TestCaseStatus::non_success(NonSuccessKind::Failure);
    status
        .set_message(message)
        .set_type(kind)
        .set_description(body);
    let mut case = TestCase::new(name, status);
    case.set_classname(format!("easement.{kind}"));
    case
}

/// JUnit XML of a verdict: one failed test case per violation and per gap in
/// `gaps` (message, how to close it), with `out` as what the run says of itself.
fn junit(verdict: &Verdict, gaps: &[(String, String)], out_lines: &[String]) -> String {
    let mut cases: Vec<TestCase> = Vec::new();
    for v in &verdict.new {
        let mut body = v.message.clone();
        if let Some(because) = &v.because {
            body.push_str(&format!("\nbecause: {because}"));
        }
        cases.push(failed(
            v.rule.as_str(),
            format!("{}:{}", v.file, v.line),
            v.message.clone(),
            body,
        ));
    }
    for k in &verdict.stale {
        let message = stale_message(k);
        cases.push(failed(
            STALE,
            stale_description(k),
            message.clone(),
            message,
        ));
    }
    for (message, close) in gaps {
        cases.push(failed(
            "evidence",
            message.clone(),
            message.clone(),
            format!("{message}\nto close: {close}"),
        ));
    }
    if cases.is_empty() {
        let mut passed = TestCase::new("architecture", TestCaseStatus::success());
        passed.set_classname("easement");
        cases.push(passed);
    }
    let mut suite = TestSuite::new("easement");
    suite.add_test_cases(cases);
    if !out_lines.is_empty() {
        suite.set_system_out(out_lines.join("\n"));
    }
    let mut report = quick_junit::Report::new("easement");
    report.add_test_suite(suite);
    report.to_string().expect("report serializes")
}

const STALE: &str = "stale-baseline";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_graph_answer_passes_the_gaps_that_hide_dependencies() {
        let stats = Stats {
            uncovered: vec!["a.go".to_string()],
            parts_partial: vec![Unjudged::new(
                "a `mod` file is not in the project".to_string(),
            )],
            symbols_partial: vec![Unjudged::new("`forbid_symbol` over kotlin".to_string())],
            std_unknown: vec!["rb".to_string()],
            ..Stats::default()
        };
        let unread = ["a.go".to_string(), "accepted.go".to_string()];
        let gaps = graph_gaps(&stats, &unread);
        let kinds: Vec<GapKind> = gaps.iter().map(|gap| gap.kind).collect();
        assert_eq!(kinds, [GapKind::Parts, GapKind::Uncovered]);
        assert_eq!(gaps[1].files, unread);
        assert_eq!(evidence_gaps(&stats).len(), 4);
    }

    /// A gap that knows its places lists them as `` file:line (`specifier`) ``
    /// instead of its files, cut where the list of files is.
    #[test]
    fn a_gap_with_places_lists_them_cut_like_files() {
        let places = (1..=FILES_SHOWN + 2).map(|line| Place {
            file: "a/a.go".to_string(),
            line,
            specifier: Some("example.com/b".to_string()),
        });
        let unjudged = Unjudged::new("the indexer did not resolve".to_string())
            .in_files(["a/a.go"])
            .at(places);
        let gap = EvidenceGap::of(GapKind::UnresolvedImports, &unjudged, "build it");
        let said = gap_lines(&[gap], &[], "unchecked", false, false);
        let listed: Vec<&str> = said.lines().skip(3).collect();
        assert_eq!(listed.len(), FILES_SHOWN + 1, "{said}");
        assert_eq!(listed[0], "    a/a.go:1 (`example.com/b`)");
        assert_eq!(
            listed[FILES_SHOWN - 1],
            format!("    a/a.go:{FILES_SHOWN} (`example.com/b`)")
        );
        assert_eq!(
            listed[FILES_SHOWN],
            "    … and 2 more (all of them in `--format json`)"
        );
        assert!(!said.contains("    a/a.go\n"), "{said}");
    }

    use crate::baseline::Verdict;

    fn verdict() -> Verdict {
        Verdict {
            new: vec![Violation {
                file: "src/a.ts".into(),
                line: 3,
                message: "a -> b is not allowed <x> 100%".into(),
                because: Some("layers".into()),
                from: "a".into(),
                to: "b".into(),
                target: "src/b.ts".into(),
                specifier: "../b".into(),
                ..Violation::of(RuleId::Dependency)
            }],
            warnings: vec![],
            owners: BTreeMap::new(),
            stale: vec![Key {
                rule: RuleId::Metric,
                file: "f".into(),
                target: "max_fan_out".into(),
                from: "core".into(),
                to: String::new(),
            }],
            legalized: 0,
        }
    }

    #[test]
    fn sarif_has_located_results() {
        let report: serde_json::Value =
            serde_json::from_str(&sarif(&verdict(), Vec::new())).unwrap();
        assert_eq!(report["version"], "2.1.0");
        let results = report["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["ruleId"], "dependency");
        assert_eq!(
            results[0]["locations"][0]["physicalLocation"]["region"]["startLine"],
            3
        );
        assert_eq!(results[1]["ruleId"], "stale-baseline");
        assert_eq!(
            report["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn github_annotations_are_escaped() {
        let out = Report {
            verdict: &verdict(),
            stats: &Stats::default(),
            coverage: &Coverage::default(),
            diff: None,
            require_evidence: false,
        }
        .github();
        assert!(out.starts_with(
            "::error file=src/a.ts,line=3,title=easement dependency::a -> b is not allowed <x> 100%25 (because: layers)\n"
        ));
        assert!(out.contains("::error title=easement stale-baseline::"));
        assert!(out.contains("easement failed"));
    }

    #[test]
    fn junit_counts_failures_and_escapes() {
        let xml = junit(&verdict(), &[], &[]);
        assert!(xml.contains(
            "<testsuite name=\"easement\" tests=\"2\" skipped=\"0\" errors=\"0\" failures=\"2\">"
        ));
        assert!(xml.contains("&lt;x&gt;"));
        let clean = junit(
            &Verdict {
                new: vec![],
                warnings: vec![],
                owners: BTreeMap::new(),
                stale: vec![],
                legalized: 0,
            },
            &[],
            &[],
        );
        assert!(clean.contains("tests=\"1\" skipped=\"0\" errors=\"0\" failures=\"0\""));
        assert!(clean.contains("<testcase name=\"architecture\" classname=\"easement\"/>"));
    }

    #[test]
    fn a_cache_hit_is_named_with_how_to_index_again() {
        let stats = Stats {
            indexed: 1,
            from_cache: 2,
            ..Stats::default()
        };
        let report: serde_json::Value = serde_json::from_str(
            &Report {
                verdict: &verdict(),
                stats: &stats,
                coverage: &Coverage::default(),
                diff: None,
                require_evidence: false,
            }
            .json(),
        )
        .unwrap();
        let cached = report["cached"].as_str().unwrap();
        assert!(
            cached.contains("2 index(es)") && cached.contains("--no-cache"),
            "{cached}"
        );
        let out = Report {
            verdict: &verdict(),
            stats: &stats,
            coverage: &Coverage::default(),
            diff: None,
            require_evidence: false,
        }
        .text();
        assert!(out.contains(cached), "{out}");

        let fresh: serde_json::Value = serde_json::from_str(
            &Report {
                verdict: &verdict(),
                stats: &Stats::default(),
                coverage: &Coverage::default(),
                diff: None,
                require_evidence: false,
            }
            .json(),
        )
        .unwrap();
        assert!(fresh.get("cached").is_none());
        assert!(
            !Report {
                verdict: &verdict(),
                stats: &Stats::default(),
                coverage: &Coverage::default(),
                diff: None,
                require_evidence: false,
            }
            .text()
            .contains("--no-cache")
        );
    }

    #[test]
    fn an_accepted_gap_leaves_the_verdict_full_and_is_still_reported() {
        let accepted = vec![AcceptedGap {
            uncovered_ok: vec!["gen/**".into(), "tools/**".into()],
            because: Some("generated".into()),
            files: vec!["gen/b.go".into(), "tools/a.go".into()],
        }];
        let coverage = Coverage {
            accepted_gaps: accepted,
            ..Coverage::default()
        };
        assert_eq!(coverage.confidence(), Confidence::Full);
        assert!(coverage.narrowed_reasons().is_empty());
        let out = Report {
            verdict: &verdict(),
            stats: &Stats::default(),
            coverage: &coverage,
            diff: None,
            require_evidence: false,
        }
        .text();
        assert!(
            out.contains("  gen/**, tools/** because: generated\n    gen/b.go\n    tools/a.go\n"),
            "{out}"
        );
    }

    #[test]
    fn narrowing_by_component_and_scope_is_one_reason_after_the_diff() {
        let coverage = Coverage {
            narrowed: vec![
                Narrowing {
                    kind: NarrowingKind::Diff,
                    limited_to: "main".into(),
                },
                Narrowing {
                    kind: NarrowingKind::Component,
                    limited_to: "db".into(),
                },
                Narrowing {
                    kind: NarrowingKind::Scope,
                    limited_to: "internal".into(),
                },
            ],
            ..Coverage::default()
        };
        assert_eq!(coverage.confidence(), Confidence::Partial);
        assert_eq!(
            coverage.narrowed_reasons(),
            [
                "only violations on lines changed since main, or caused by the change to the policy, were judged, the rest of the project was not",
                "the report is limited to component db, path internal, the rest of the project was not judged",
            ]
        );
    }

    #[test]
    fn require_evidence_turns_the_gaps_of_a_clean_run_into_errors() {
        let clean = Verdict {
            new: vec![],
            warnings: vec![],
            owners: BTreeMap::new(),
            stale: vec![],
            legalized: 0,
        };
        let stats = Stats {
            uncovered: vec!["tools/gen.go".into()],
            ..Stats::default()
        };
        let coverage = Coverage::of_check(&stats, vec![], vec![]);
        let report = |require_evidence| Report {
            verdict: &clean,
            stats: &stats,
            coverage: &coverage,
            diff: None,
            require_evidence,
        };
        assert!(report(false).passed());
        let github = report(false).github();
        assert!(
            github.contains("::warning title=easement partial::"),
            "{github}"
        );
        assert!(github.contains("easement passed"), "{github}");

        assert!(!report(true).passed());
        let github = report(true).github();
        assert!(
            github.contains("::error title=easement partial::"),
            "{github}"
        );
        assert!(github.contains("easement failed"), "{github}");
        // Nothing to require of a run that reads everything.
        let whole = Coverage::default();
        assert!(
            Report {
                coverage: &whole,
                ..report(true)
            }
            .passed()
        );
    }

    /// SARIF and JUnit say what a run leaves out too: a clean result over code no
    /// indexer read is not a clean bill.
    #[test]
    fn sarif_and_junit_say_the_gaps_the_narrowing_and_the_cache() {
        let stats = Stats {
            from_cache: 1,
            uncovered: vec!["tools/gen.go".into()],
            ..Stats::default()
        };
        let coverage = Coverage {
            narrowed: super::narrowed(None, &["web".into()], &[]),
            ..Coverage::of_check(&stats, vec![], vec![])
        };
        let clean = Verdict {
            new: vec![],
            warnings: vec![],
            owners: BTreeMap::new(),
            stale: vec![],
            legalized: 0,
        };
        let report = |require_evidence| Report {
            verdict: &clean,
            stats: &stats,
            coverage: &coverage,
            diff: None,
            require_evidence,
        };
        let sarif: serde_json::Value = serde_json::from_str(&report(false).sarif()).unwrap();
        let said: Vec<(String, String)> =
            sarif["runs"][0]["invocations"][0]["toolExecutionNotifications"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| {
                    (
                        n["level"].as_str().unwrap().to_string(),
                        n["message"]["text"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
        assert_eq!(said.len(), 3, "{said:?}");
        assert_eq!(said[0].0, "note");
        assert!(said[0].1.contains("limited to component web"), "{said:?}");
        assert_eq!(said[1].0, "warning");
        assert!(said[1].1.contains("tools/gen.go"), "{said:?}");
        assert!(said[2].1.contains("--no-cache"), "{said:?}");
        let required: serde_json::Value = serde_json::from_str(&report(true).sarif()).unwrap();
        assert_eq!(
            required["runs"][0]["invocations"][0]["toolExecutionNotifications"][1]["level"],
            "error"
        );

        let xml = report(false).junit();
        assert!(xml.contains("<system-out>"), "{xml}");
        assert!(xml.contains("tools/gen.go"), "{xml}");
        assert!(xml.contains("--no-cache"), "{xml}");
        assert!(xml.contains("failures=\"0\""), "{xml}");
        // Under `--require-evidence` a gap fails the run, so it is a failed test case.
        let xml = report(true).junit();
        assert!(xml.contains("failures=\"1\""), "{xml}");
        assert!(xml.contains("classname=\"easement.evidence\""), "{xml}");
    }

    #[test]
    fn github_says_why_a_verdict_is_partial_and_when_it_was_cached() {
        let stats = Stats {
            from_cache: 1,
            uncovered: vec!["tools/gen.go".into()],
            ..Stats::default()
        };
        let out = Report {
            verdict: &verdict(),
            stats: &stats,
            coverage: &Coverage::of_check(&stats, vec![], vec![]),
            diff: None,
            require_evidence: false,
        }
        .github();
        assert!(
            out.contains(
                "::warning title=easement partial::1 source files were not covered by an indexer"
            ),
            "{out}"
        );
        assert!(out.contains("::notice title=easement cache::1 index(es) came from the cache"));
        assert!(out.contains("--no-cache"));
        assert!(
            out.ends_with("1 files not covered by an indexer\n"),
            "{out}"
        );
        assert!(
            !Report {
                verdict: &verdict(),
                stats: &Stats::default(),
                coverage: &Coverage::default(),
                diff: None,
                require_evidence: false,
            }
            .github()
            .contains("::warning")
        );

        // A run narrowed by design is a notice: a warning on every pull request
        // would teach to skip the ones about code nobody read.
        let narrowed = Coverage {
            narrowed: super::narrowed(None, &["web".into()], &[]),
            ..Coverage::default()
        };
        let out = Report {
            verdict: &verdict(),
            stats: &Stats::default(),
            coverage: &narrowed,
            diff: None,
            require_evidence: false,
        }
        .github();
        assert!(
            out.contains("::notice title=easement partial::the report is limited to component web"),
            "{out}"
        );
        assert!(!out.contains("::warning"), "{out}");
    }
}
