//! The startup block. "Why is Ada not there" should be answerable from the
//! first eight lines of output, so this prints where every moving part came
//! from.

use crate::clock::{format_offset, Clock, SKEW_VAR};
use crate::persona::Origin;
use crate::registry::{SourceState, Warning};

/// Everything the banner says, gathered by the caller so this stays a pure
/// function that tests can assert on.
pub struct Banner<'a> {
    pub issuer: &'a str,
    /// The persona picker. Phase 1 deferred this line on the grounds that the
    /// banner must not print a URL that 404s; Phase 4 is the phase where it
    /// stops doing that.
    pub ui: &'a str,
    /// The live request log. Same rule as `ui`: Phase 1 deferred a line the
    /// page for which did not exist yet, and this is the phase where it does.
    pub log: &'a str,
    pub listen: &'a str,
    pub data_dir: &'a str,
    pub kid: &'a str,
    /// **Only printed when it is wrong.** A dev tool may lie about the time; it
    /// may not do so quietly — so a skewed process says so on the first screen
    /// of output, and names what the lie does to a 60-second token.
    pub clock: Clock,
    /// **One line each**, in precedence order — the built-ins or the global
    /// file, then every linked project. A source that gave nothing is on the
    /// list saying why rather than omitted: "why is Ada not there" has to stay
    /// answerable from the first eight lines of output, and after Phase 7 the
    /// answer is as often "that project is not where the registry says" as it
    /// is "you have a users.yaml".
    pub sources: &'a [SourceState],
    /// Everything wrong that is not any one source's fault — a shadowed id, a
    /// registry file that will not parse.
    pub warnings: &'a [Warning],
}

pub fn render(b: &Banner) -> String {
    format!(
        "lanyard {}\n  \
         Issuer    → {}\n  \
         UI        → {}\n  \
         Log       → {}\n  \
         Listening → {}\n  \
         Data dir  → {}\n  \
         Signing   → kid {}\n\
         {}{}{}",
        env!("CARGO_PKG_VERSION"),
        b.issuer,
        b.ui,
        b.log,
        b.listen,
        b.data_dir,
        b.kid,
        clock_block(b.clock),
        personas_block(b.sources),
        warnings_block(b.sources, b.warnings),
    )
}

/// The `Clock →` line, and **nothing at all** when the clock is the machine's.
///
/// An unskewed run is every run, and a line that says "normal" on every run is
/// a line nobody reads — which is the same argument that keeps the warning band
/// off a page with no warnings.
fn clock_block(clock: Clock) -> String {
    if !clock.is_skewed() {
        return String::new();
    }
    // The consequence, not just the number: "skewed -5m0s" leaves the reader to
    // do the arithmetic that is the entire reason the line exists.
    let offset = format_offset(clock.skew_seconds());
    let consequence = if clock.skew_seconds() < 0 {
        format!(
            "tokens minted here are already expired by {} for anything\n\
             \x20             on a correct clock",
            format_offset(clock.skew_seconds().saturating_add(60).min(0)).trim_start_matches('-'),
        )
    } else {
        "tokens minted here are not valid yet for anything\n\
         \x20             on a correct clock"
            .to_string()
    };
    format!("  Clock     → skewed {offset} ({SKEW_VAR}) — {consequence}\n")
}

/// "Why is Ada not there" is answered here: every source, in the order the
/// precedence ladder reads them, with what each one gave.
fn personas_block(sources: &[SourceState]) -> String {
    let mut out = String::new();
    for (i, source) in sources.iter().enumerate() {
        // The first source keeps the label; the rest hang under it, aligned, so
        // the block reads as one answer to one question.
        let label = if i == 0 {
            "Personas  → "
        } else {
            // Aligned under the first line's value. `→` is one column and three
            // bytes, so this is counted in columns and asserted in columns.
            "            "
        };
        out.push_str(&format!("  {label}{}\n", source_line(source)));
    }
    out
}

fn source_line(source: &SourceState) -> String {
    let name = match &source.origin {
        Origin::BuiltIn => "built-in defaults".to_string(),
        Origin::File(path) | Origin::Project(path) => path.display().to_string(),
    };
    match &source.error {
        Some(error) => format!("{name} — {error}"),
        None => format!("{name} ({})", source.ids.join(", ")),
    }
}

/// The warnings **that are not already on a source's own line** — a shadowed
/// id, a registry file that will not parse. A broken project already said what
/// was wrong where it was listed, and saying it twice in eleven lines teaches a
/// reader to skim the block.
fn warnings_block(sources: &[SourceState], warnings: &[Warning]) -> String {
    warnings
        .iter()
        .filter(|warning| {
            !sources
                .iter()
                .any(|source| source.error.as_deref() == Some(warning.0.as_str()))
        })
        .map(|warning| format!("  Warning   → {warning}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        sample_with(
            &[SourceState {
                origin: Origin::BuiltIn,
                ids: vec!["ada".into(), "mira".into(), "nobody".into()],
                error: None,
            }],
            &[],
        )
    }

    fn sample_with(sources: &[SourceState], warnings: &[Warning]) -> String {
        sample_clocked(Clock::real(), sources, warnings)
    }

    fn sample_clocked(clock: Clock, sources: &[SourceState], warnings: &[Warning]) -> String {
        render(&Banner {
            issuer: "http://lanyard:9500/oidc",
            ui: "http://lanyard:9500/_/",
            log: "http://lanyard:9500/_/log",
            listen: "0.0.0.0:9500",
            data_dir: "/tmp/lanyard-data",
            kid: "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs",
            clock,
            sources,
            warnings,
        })
    }

    fn builtin() -> [SourceState; 1] {
        [SourceState {
            origin: Origin::BuiltIn,
            ids: vec!["ada".into()],
            error: None,
        }]
    }

    fn line_with(out: &str, label: &str) -> String {
        out.lines()
            .find(|l| l.contains(label))
            .unwrap_or_else(|| panic!("no {label} line in:\n{out}"))
            .to_string()
    }

    #[test]
    fn the_issuer_line_carries_the_issuer_verbatim() {
        let out = sample();
        assert!(
            line_with(&out, "Issuer").ends_with("http://lanyard:9500/oidc"),
            "issuer must appear byte-identical; discovery is compared against it\n{out}"
        );
    }

    #[test]
    fn the_banner_says_where_every_moving_part_came_from() {
        let out = sample();
        assert!(line_with(&out, "Listening").contains("0.0.0.0:9500"));
        assert!(line_with(&out, "UI").ends_with("http://lanyard:9500/_/"));
        assert!(line_with(&out, "Log").ends_with("http://lanyard:9500/_/log"));
        assert!(line_with(&out, "Data dir").contains("/tmp/lanyard-data"));
        assert!(line_with(&out, "Signing").contains("NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"));
    }

    /// A dev tool may lie about the time; it may not do so quietly. The line
    /// names the offset, the variable that produced it, and what it does to a
    /// 60-second token — the arithmetic is the reason the line exists.
    #[test]
    fn a_skewed_clock_gets_a_line_naming_the_variable_and_the_consequence() {
        let out = sample_clocked(Clock::skewed(-300), &builtin(), &[]);
        let line = line_with(&out, "Clock");
        assert!(line.contains("-5m0s"), "{line}");
        assert!(line.contains("LANYARD_CLOCK_SKEW"), "{line}");
        assert!(
            out.contains("already expired by 4m0s"),
            "the consequence, not just the number:\n{out}"
        );
    }

    /// Every ordinary run is unskewed, and a line that says "normal" on every
    /// run is a line nobody reads.
    #[test]
    fn an_unskewed_clock_prints_no_line_at_all() {
        let out = sample_clocked(Clock::real(), &builtin(), &[]);
        assert!(!out.contains("Clock"), "{out}");
        assert!(!out.contains("LANYARD_CLOCK_SKEW"), "{out}");
    }

    /// A clock that runs fast mints tokens nothing else will accept yet, which
    /// is a different sentence from the one a slow clock earns.
    #[test]
    fn a_clock_that_runs_fast_says_not_valid_yet() {
        let out = sample_clocked(Clock::skewed(300), &builtin(), &[]);
        let line = line_with(&out, "Clock");
        assert!(line.contains("+5m0s"), "{line}");
        assert!(out.contains("not valid yet"), "{out}");
    }

    #[test]
    fn the_personas_line_names_the_source_and_the_ids() {
        let out = sample();
        let line = line_with(&out, "Personas");
        assert!(line.contains("built-in defaults"), "{line}");
        assert!(line.contains("ada") && line.contains("nobody"), "{line}");
    }

    #[test]
    fn a_personas_file_is_named_by_path() {
        let out = sample_with(
            &[SourceState {
                origin: Origin::File("/tmp/users.yaml".into()),
                ids: vec!["solo".into()],
                error: None,
            }],
            &[],
        );
        let line = line_with(&out, "Personas");
        assert!(line.contains("/tmp/users.yaml"), "{line}");
        assert!(!line.contains("built-in"), "{line}");
    }

    /// Criterion 24. **One line per source**, so "why is Ada not there" is
    /// still answerable from the first eight lines of output — and a registry
    /// entry whose directory is gone is on that list saying so rather than
    /// being quietly omitted.
    #[test]
    fn every_source_gets_a_line_and_a_broken_one_says_why() {
        let out = sample_with(
            &[
                SourceState {
                    origin: Origin::BuiltIn,
                    ids: vec!["ada".into(), "mira".into(), "nobody".into()],
                    error: None,
                },
                SourceState {
                    origin: Origin::Project("/home/dev/code/billing/lanyard.yaml".into()),
                    ids: vec!["dev-admin".into()],
                    error: None,
                },
                SourceState {
                    origin: Origin::Project("/home/dev/code/old/lanyard.yaml".into()),
                    ids: Vec::new(),
                    error: Some("/home/dev/code/old: linked directory does not exist".into()),
                },
            ],
            &[],
        );

        assert!(line_with(&out, "built-in defaults").contains("ada, mira, nobody"));
        let billing = line_with(&out, "/home/dev/code/billing/lanyard.yaml");
        assert!(billing.contains("dev-admin"), "{billing}");
        let broken = line_with(&out, "/home/dev/code/old/lanyard.yaml");
        assert!(
            broken.contains("does not exist"),
            "a missing link is listed as missing, not omitted: {broken}"
        );
        assert!(
            out.lines().filter(|l| l.contains("lanyard.yaml")).count() == 2,
            "one line each:\n{out}"
        );
    }

    /// The block is one answer to one question, so the continuation lines are
    /// aligned under the first.
    #[test]
    fn the_source_lines_line_up_under_the_label() {
        let out = sample_with(
            &[
                SourceState {
                    origin: Origin::BuiltIn,
                    ids: vec!["ada".into()],
                    error: None,
                },
                SourceState {
                    origin: Origin::Project("/code/billing/lanyard.yaml".into()),
                    ids: vec!["dev-admin".into()],
                    error: None,
                },
            ],
            &[],
        );
        let first = line_with(&out, "built-in defaults");
        let second = line_with(&out, "/code/billing/lanyard.yaml");
        // Counted in **characters**, not bytes: `→` is three bytes wide and one
        // column, and a byte comparison here passes on a block that is visibly
        // two columns out.
        let column_of = |line: &str, needle: char| line.chars().position(|c| c == needle).unwrap();
        assert_eq!(
            column_of(&first, 'b'),
            column_of(&second, '/'),
            "aligned:\n{out}"
        );
    }

    /// A broken source already said what was wrong on its own line; saying it
    /// twice in eleven lines teaches a reader to skim the block.
    #[test]
    fn a_sources_own_error_is_not_repeated_as_a_warning() {
        let out = sample_with(
            &[SourceState {
                origin: Origin::Project("/code/old/lanyard.yaml".into()),
                ids: Vec::new(),
                error: Some("/code/old: linked directory does not exist".into()),
            }],
            &[Warning("/code/old: linked directory does not exist".into())],
        );
        assert_eq!(
            out.matches("linked directory does not exist").count(),
            1,
            "{out}"
        );
    }

    /// A shadowed id is not any one source's fault, so it gets its own line
    /// rather than being hung off whichever file happened to lose.
    #[test]
    fn a_shadow_warning_gets_its_own_line() {
        let out = sample_with(
            &[SourceState {
                origin: Origin::BuiltIn,
                ids: vec!["ada".into()],
                error: None,
            }],
            &[Warning(
                "persona \"ada\" is defined in A and B; A wins".into(),
            )],
        );
        let line = line_with(&out, "Warning");
        assert!(line.contains("is defined in A and B"), "{line}");
    }
}
