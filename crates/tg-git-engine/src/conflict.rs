//! Parsing of conflicted files for the in-app conflict editor.
//!
//! Instead of only a blanket "ours/theirs" (like `git checkout --ours/--theirs`)
//! the editor splits the file into segments along the conflict markers and thus
//! allows a per-conflict resolution (including "both", in either order) with an
//! editable result. Supports both the standard style (`merge`) and `diff3`
//! (with a base section).

use tg_domain::{ConflictFile, ConflictSegment};

/// Upper bound for files opened in the editor (text; travels over IPC).
pub(crate) const MAX_CONFLICT_BYTES: usize = 5 * 1024 * 1024;

/// Shortest marker length we accept — which is also git's default.
///
/// git allows less via `conflict-marker-size` (measured: 3 yields `<<< HEAD`).
/// We deliberately leave that alone: below 7, ordinary text lines like `===`
/// would become markers, and the case is so rare that the damage would outweigh
/// the benefit.
const MIN_MARKER: usize = 7;

/// Length of the marker run at the start of the line, if the line could be a
/// marker: `size` characters `ch`, followed by end of line or — only where git
/// appends a label (`<<<<<<< HEAD`, `>>>>>>> branch`, `||||||| <ref>`) — a
/// space. The separator `=======` never carries a label.
fn marker_run(line: &str, ch: u8, with_label: bool) -> Option<usize> {
    let b = line.as_bytes();
    let b = b.strip_suffix(b"\r").unwrap_or(b);
    let n = b.iter().take_while(|&&x| x == ch).count();
    if n < MIN_MARKER {
        return None;
    }
    if b.len() == n || (with_label && b[n] == b' ') {
        Some(n)
    } else {
        None
    }
}

/// A marker of EXACTLY the length the opening line prescribed.
fn is_marker(line: &str, ch: u8, with_label: bool, size: usize) -> bool {
    marker_run(line, ch, with_label) == Some(size)
}

fn context(lines: Vec<String>) -> ConflictSegment {
    ConflictSegment {
        kind: "context".into(),
        lines,
        ours: Vec::new(),
        theirs: Vec::new(),
        base: None,
    }
}

/// A fully closed conflict starting at `start`.
struct Block {
    ours: Vec<String>,
    base: Option<Vec<String>>,
    theirs: Vec<String>,
    /// Index AFTER the closing `>>>>>>>` line.
    end: usize,
}

/// Reads a conflict starting at the opening line `start` — or `None` if it does
/// not close cleanly.
///
/// The marker length comes from the OPENING line and then applies to the whole
/// block. That is the only reliable way: git picks the length when writing from
/// `conflict-marker-size`, but that attribute may have changed between the merge
/// and the read — which is exactly what happens when a branch BRINGS the
/// attribute along (git still wrote with 7 while the file afterwards prescribes
/// 12) or when `.gitattributes` is itself conflicted. The file knows better than
/// the attribute.
///
/// The requirement that the block MUST close completely replaces the former
/// setext guard: an underline `========` inside the ours block has a different
/// length than the opening line and therefore does not separate.
fn scan_block(raw: &[&str], start: usize, size: usize) -> Option<Block> {
    let mut i = start + 1;
    let mut ours = Vec::new();
    while i < raw.len()
        && !is_marker(raw[i], b'|', true, size)
        && !is_marker(raw[i], b'=', false, size)
    {
        ours.push(raw[i].to_string());
        i += 1;
    }

    // Optional base section (diff3).
    let mut base = None;
    if i < raw.len() && is_marker(raw[i], b'|', true, size) {
        i += 1;
        let mut b = Vec::new();
        while i < raw.len() && !is_marker(raw[i], b'=', false, size) {
            b.push(raw[i].to_string());
            i += 1;
        }
        base = Some(b);
    }

    // Without a separator this is not a conflict but text that happened to start that way.
    if i >= raw.len() {
        return None;
    }
    i += 1;

    let mut theirs = Vec::new();
    while i < raw.len() && !is_marker(raw[i], b'>', true, size) {
        theirs.push(raw[i].to_string());
        i += 1;
    }
    // Same when there is no closing marker.
    if i >= raw.len() {
        return None;
    }

    Some(Block {
        ours,
        base,
        theirs,
        end: i + 1,
    })
}

/// Splits the file content into context and conflict segments.
pub(crate) fn parse(file: &str, content: &str) -> ConflictFile {
    // Detect the EOL (dominant line ending) for lossless saving.
    let crlf = content.matches("\r\n").count() * 2 >= content.matches('\n').count().max(1);
    let eol = if crlf { "crlf" } else { "lf" };

    // Line by line without the line ending; a trailing empty line caused by a
    // trailing \n is not counted as its own line (the way an editor shows it).
    let normalized = content.replace("\r\n", "\n");
    let mut raw: Vec<&str> = normalized.split('\n').collect();
    if raw.last() == Some(&"") {
        raw.pop();
    }

    let mut segments: Vec<ConflictSegment> = Vec::new();
    let mut ctx: Vec<String> = Vec::new();
    let mut has_conflicts = false;

    let mut i = 0;
    while i < raw.len() {
        let block = marker_run(raw[i], b'<', true).and_then(|size| scan_block(&raw, i, size));
        let Some(block) = block else {
            ctx.push(raw[i].to_string());
            i += 1;
            continue;
        };
        // Close the running context.
        if !ctx.is_empty() {
            segments.push(context(std::mem::take(&mut ctx)));
        }
        has_conflicts = true;
        segments.push(ConflictSegment {
            kind: "conflict".into(),
            lines: Vec::new(),
            ours: block.ours,
            theirs: block.theirs,
            base: block.base,
        });
        i = block.end;
    }
    if !ctx.is_empty() {
        segments.push(context(ctx));
    }

    ConflictFile {
        file: file.to_string(),
        segments,
        eol: eol.to_string(),
        has_conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_conflict() {
        let c = "before\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> branch\nafter\n";
        let f = parse("a.txt", c);
        assert!(f.has_conflicts);
        assert_eq!(f.eol, "lf");
        assert_eq!(f.segments.len(), 3);
        assert_eq!(f.segments[0].kind, "context");
        assert_eq!(f.segments[0].lines, vec!["before"]);
        assert_eq!(f.segments[1].kind, "conflict");
        assert_eq!(f.segments[1].ours, vec!["mine"]);
        assert_eq!(f.segments[1].theirs, vec!["theirs"]);
        assert_eq!(f.segments[1].base, None);
        assert_eq!(f.segments[2].lines, vec!["after"]);
    }

    #[test]
    fn parses_diff3_with_base_and_crlf() {
        let c =
            "<<<<<<< HEAD\r\nmine\r\n||||||| base\r\norig\r\n=======\r\ntheirs\r\n>>>>>>> b\r\n";
        let f = parse("a.txt", c);
        assert_eq!(f.eol, "crlf");
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].ours, vec!["mine"]);
        assert_eq!(
            f.segments[0].base.as_deref(),
            Some(&["orig".to_string()][..])
        );
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn no_markers_no_conflicts() {
        let f = parse("a.txt", "just\ntext\n");
        assert!(!f.has_conflicts);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].lines, vec!["just", "text"]);
    }

    #[test]
    fn setext_underline_is_not_a_separator() {
        // A Markdown/RST underline `========` (8 characters) inside the ours
        // block is content — only the exact `=======` separates ours from theirs.
        let c = "<<<<<<< HEAD\nTitle\n========\n=======\ntheirs\n>>>>>>> b\n";
        let f = parse("a.md", c);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].ours, vec!["Title", "========"]);
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn incomplete_block_stays_content() {
        // A line of marker characters without a matching separator AND closing
        // marker is text — exactly the case the previously hard-wired 7-character
        // comparison was meant to catch.
        let c = "<<<<<<<<\nTitle\n========\n";
        let f = parse("a.txt", c);
        assert!(!f.has_conflicts);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].lines, vec!["<<<<<<<<", "Title", "========"]);
    }

    #[test]
    fn complete_block_counts_at_eight_characters_too() {
        // DELIBERATE change against the earlier version: if the block closes
        // cleanly at the same length it is a conflict — because that is exactly
        // how git writes it with conflict-marker-size=8. The earlier version
        // assumed content here and was blind for EVERY differing marker length;
        // that was the more expensive mistake.
        let c = "<<<<<<<<\nmine\n========\ntheirs\n>>>>>>>>\n";
        let f = parse("a.txt", c);
        assert!(f.has_conflicts);
        assert_eq!(f.segments[0].ours, vec!["mine"]);
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn separator_with_a_suffix_is_content() {
        // git writes the separator without a label — `======= x` is content;
        // so is `|||||||x` (8th character is not a space).
        let c = "<<<<<<< HEAD\n======= x\n|||||||x\n=======\ntheirs\n>>>>>>> b\n";
        let f = parse("a.txt", c);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].ours, vec!["======= x", "|||||||x"]);
        assert_eq!(f.segments[0].base, None);
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn larger_marker_length_is_detected() {
        // .gitattributes `a.md conflict-marker-size=12` -> git writes 12-character
        // markers (verified empirically against git). With a fixed 7 the file
        // stayed marker-less and the workshop reported it as probably already
        // resolved.
        let c = "<<<<<<<<<<<< HEAD\nmine\n============\ntheirs\n>>>>>>>>>>>> b\n";
        let f = parse("a.md", c);
        assert!(f.has_conflicts);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].ours, vec!["mine"]);
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn marker_lengths_below_seven_stay_content() {
        // git also allows 3 via conflict-marker-size (measured). We deliberately
        // leave that alone: below 7, `===` would become a separator in any file
        // whatsoever. The case is so rare that the damage would outweigh the
        // benefit.
        let c = "<<< HEAD\nmine\n===\ntheirs\n>>> b\n";
        let f = parse("a.txt", c);
        assert!(!f.has_conflicts);
    }

    #[test]
    fn only_the_opening_lines_length_counts() {
        // The opening line prescribes 12 — so the 7-character line is content.
        // The setext guard works in BOTH directions.
        let c = "<<<<<<<<<<<< HEAD\nTitle\n=======\n============\ntheirs\n>>>>>>>>>>>> b\n";
        let f = parse("a.md", c);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].ours, vec!["Title", "======="]);
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn diff3_with_large_marker_length() {
        let c = "<<<<<<<<<<<< HEAD\nmine\n|||||||||||| base\norig\n============\ntheirs\n>>>>>>>>>>>> b\n";
        let f = parse("a.md", c);
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].ours, vec!["mine"]);
        assert_eq!(
            f.segments[0].base.as_deref(),
            Some(&["orig".to_string()][..])
        );
        assert_eq!(f.segments[0].theirs, vec!["theirs"]);
    }

    #[test]
    fn seven_marker_despite_differing_attribute() {
        // Regression B1: if a branch BRINGS `.gitattributes` with
        // conflict-marker-size=12 along, git still merged the file with 7 — the
        // attribute did not apply at merge time. Whoever reads the length from
        // the attribute is blind here. The file knows better: the opening line
        // prescribes the length.
        let c = "<<<<<<< HEAD\nfrom main\n=======\nfrom side\n>>>>>>> side\n";
        let f = parse("docs/a.md", c);
        assert!(f.has_conflicts);
        assert_eq!(f.segments[0].ours, vec!["from main"]);
        assert_eq!(f.segments[0].theirs, vec!["from side"]);
    }

    #[test]
    fn mixed_document_stays_complete() {
        // Roundtrip view: context before/after the conflict including a
        // `========` line outside of it stays fully preserved as context.
        let c = "head\n<<<<<<< HEAD\nTitle\n========\n=======\ntheirs\n>>>>>>> b\nmiddle\n========\nfoot\n";
        let f = parse("a.md", c);
        assert!(f.has_conflicts);
        assert_eq!(f.segments.len(), 3);
        assert_eq!(f.segments[0].lines, vec!["head"]);
        assert_eq!(f.segments[1].ours, vec!["Title", "========"]);
        assert_eq!(f.segments[1].theirs, vec!["theirs"]);
        assert_eq!(f.segments[2].lines, vec!["middle", "========", "foot"]);
    }
}
