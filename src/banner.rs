//! The startup block. "Why is Ada not there" should be answerable from the
//! first eight lines of output, so this prints where every moving part came
//! from.

use crate::persona::{Origin, Personas};

/// Everything the banner says, gathered by the caller so this stays a pure
/// function that tests can assert on.
pub struct Banner<'a> {
    pub issuer: &'a str,
    pub listen: &'a str,
    pub data_dir: &'a str,
    pub kid: &'a str,
    pub personas: &'a Personas,
}

pub fn render(b: &Banner) -> String {
    format!(
        "lanyard {}\n  \
         Issuer    → {}\n  \
         Listening → {}\n  \
         Data dir  → {}\n  \
         Signing   → kid {}\n  \
         Personas  → {}\n",
        env!("CARGO_PKG_VERSION"),
        b.issuer,
        b.listen,
        b.data_dir,
        b.kid,
        personas_line(b.personas),
    )
}

/// "Why is Ada not there" is answered here: which of the two sources was used,
/// and when it is the file, its path.
fn personas_line(p: &Personas) -> String {
    let ids: Vec<&str> = p.list.iter().map(|p| p.id.as_str()).collect();
    match &p.origin {
        Origin::BuiltIn => format!("built-in defaults ({})", ids.join(", ")),
        Origin::File(path) => format!("{} ({})", path.display(), ids.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        sample_with(&Personas::builtin())
    }

    fn sample_with(personas: &Personas) -> String {
        render(&Banner {
            issuer: "http://lanyard:9500/oidc",
            listen: "0.0.0.0:9500",
            data_dir: "/tmp/lanyard-data",
            kid: "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs",
            personas,
        })
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
        assert!(line_with(&out, "Data dir").contains("/tmp/lanyard-data"));
        assert!(line_with(&out, "Signing").contains("NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"));
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
        let personas = Personas::parse(
            "personas:\n  - id: solo\n",
            std::path::Path::new("/tmp/users.yaml"),
        )
        .unwrap();
        let line = line_with(&sample_with(&personas), "Personas");
        assert!(line.contains("/tmp/users.yaml"), "{line}");
        assert!(!line.contains("built-in"), "{line}");
    }
}
