//! The rule R2 asks for, enforced against the source rather than remembered.
//!
//! `mirror-provenance` gives effective-k exactly one renderer — `Quotation`,
//! which cannot be constructed without a `Bracket` — so the only way this tool
//! can print a bare figure is by reaching past that renderer into the raw field
//! and formatting it itself. That is a small, specific thing to do, and it is
//! also the obvious thing to do while adding a line to some output in a hurry.
//!
//! So the check is on the shape of the code. The fields stay public because
//! `bootstrap`, `crowd` and `compare` all compute with them; what is refused is
//! putting one inside something that prints. A test rather than a convention,
//! for the same reason the verifying key has `vk_drift.rs`: a rule nothing
//! checks is a rule that has already been broken somewhere nobody looked.

use std::path::Path;

/// The fields that are a figure rather than an input to one.
const FIGURES: [&str; 2] = ["eff_k_shannon", "eff_k_min_entropy"];

/// Where a value ends up in front of a reader.
const PRINTERS: [&str; 4] = ["println!", "print!", "eprintln!", "push_str"];

/// `crowd.rs` renders `docs/CROWD.md`, and its table carries the bracket in
/// adjacent columns — checked by `the_crowd_table_carries_the_bracket` below
/// rather than taken on faith. Markdown is not something `Quotation`'s plain
/// text can produce, so this is the one place allowed to lay the figures out
/// itself.
const ALLOWED: [&str; 1] = ["crowd.rs"];

fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the crate's src directory must be readable") {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a UTF-8 file name")
            .to_string();
        out.push((
            name,
            std::fs::read_to_string(&path).expect("a readable source file"),
        ));
    }
    assert!(
        out.len() > 5,
        "the source sweep found almost nothing to read"
    );
    out
}

/// No effective-k reaches a reader except through the renderer that carries its
/// bracket.
#[test]
fn no_module_prints_an_effective_k_by_hand() {
    let mut offences = Vec::new();

    for (name, text) in sources() {
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        // A print can span several lines, so the unit is the statement rather
        // than the line: from a printing macro to the semicolon that ends it.
        for (i, line) in text.lines().enumerate() {
            if !PRINTERS.iter().any(|p| line.contains(p)) {
                continue;
            }
            let mut statement = String::new();
            for l in text.lines().skip(i) {
                statement.push_str(l);
                statement.push('\n');
                if l.trim_end().ends_with(");") {
                    break;
                }
            }
            for figure in FIGURES {
                if statement.contains(figure) {
                    offences.push(format!("{name}:{}: {figure}", i + 1));
                }
            }
        }
    }

    assert!(
        offences.is_empty(),
        "an effective-k is being formatted for output without the bracket that makes it \
         readable:\n  {}\n\nPrint a `mirror_provenance::Quotation` instead — it takes a \
         `Bracket` to build, so the range cannot be left behind.",
        offences.join("\n  ")
    );
}

/// The one exemption, and the reason it is allowed.
#[test]
fn the_crowd_table_carries_the_bracket() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/crowd.rs"))
        .expect("crowd.rs must be readable");

    assert!(
        text.contains("unresolved bracket"),
        "crowd.rs is exempt from the sweep because its table shows the bracket in its own \
         columns. That column heading is gone, so the exemption no longer holds."
    );
    for end in ["b.lower.eff_k_shannon", "b.upper.eff_k_shannon"] {
        assert!(
            text.contains(end),
            "crowd.rs prints an effective-k without {end}, so the table's range is not the \
             bracket it claims to be."
        );
    }
}
