//! Typst AST-aware prose range utilities.
//!
//! Shared between the extract validation pass and the render pipeline.
//! Walks the typst-syntax AST to identify byte ranges of prose text
//! (Text and Space nodes), excluding headings, code, math, labels,
//! refs, links, and function calls.

use typst_syntax::{SyntaxKind, SyntaxNode, parse};

/// Collect byte ranges of prose text in a Typst source.
///
/// Walks the AST using the same skip rules as the extractor (headings,
/// code, math, labels, refs, links, function calls) and returns sorted
/// byte ranges of `Text` and `Space` nodes — the only positions where
/// index markers can safely be inserted.
pub fn collect_prose_ranges(source: &str) -> Vec<(usize, usize)> {
    let root = parse(source);
    let mut ranges = Vec::new();
    walk_for_ranges(&root, 0, &mut ranges, false);
    // Merge adjacent ranges so multi-word terms spanning text+space+text match.
    merge_ranges(&mut ranges);
    ranges
}

fn walk_for_ranges(
    node: &SyntaxNode,
    base_offset: usize,
    ranges: &mut Vec<(usize, usize)>,
    in_heading: bool,
) {
    // Skip entirely — same nodes the extractor skips.
    if matches!(
        node.kind(),
        SyntaxKind::Raw
            | SyntaxKind::Equation
            | SyntaxKind::Label
            | SyntaxKind::Ref
            | SyntaxKind::RefMarker
            | SyntaxKind::Link
            | SyntaxKind::LineComment
            | SyntaxKind::BlockComment
            | SyntaxKind::Hash
    ) {
        return;
    }

    if node.kind() == SyntaxKind::Heading {
        let mut child_offset = base_offset;
        for child in node.children() {
            walk_for_ranges(child, child_offset, ranges, true);
            child_offset += child.len();
        }
        return;
    }

    match node.kind() {
        SyntaxKind::Text | SyntaxKind::Space if !in_heading => {
            let len = node.len();
            if len > 0 {
                ranges.push((base_offset, base_offset + len));
            }
        }
        _ => {
            let mut child_offset = base_offset;
            for child in node.children() {
                walk_for_ranges(child, child_offset, ranges, in_heading);
                child_offset += child.len();
            }
        }
    }
}

/// Merge adjacent/overlapping ranges into contiguous spans.
fn merge_ranges(ranges: &mut Vec<(usize, usize)>) {
    if ranges.len() <= 1 {
        return;
    }
    ranges.sort_by_key(|r| r.0);
    let mut write = 0;
    for read in 1..ranges.len() {
        if ranges[read].0 <= ranges[write].1 {
            ranges[write].1 = ranges[write].1.max(ranges[read].1);
        } else {
            write += 1;
            ranges[write] = ranges[read];
        }
    }
    ranges.truncate(write + 1);
}

/// Find a term in Typst source, only matching within prose text.
///
/// Parses the AST to determine safe byte ranges, then searches
/// the original source — skipping matches inside labels, links,
/// function arguments, code, math, and headings.
pub fn find_term_offset_in_prose(
    source: &str,
    term: &str,
    prose_ranges: &[(usize, usize)],
) -> Option<usize> {
    let lower_source = source.to_lowercase();
    let lower_term = term.to_lowercase();

    let mut start = 0;
    while let Some(pos) = lower_source[start..].find(&lower_term) {
        let abs_pos = start + pos;
        let abs_end = abs_pos + lower_term.len();

        // Check if match falls entirely within a merged prose range.
        let in_prose = prose_ranges
            .iter()
            .any(|&(rs, re)| abs_pos >= rs && abs_end <= re);

        if in_prose {
            // Guard: offset from lowered buffer may not be valid in the original
            // source when Unicode case folding changes byte lengths.
            if abs_end <= source.len() && source.is_char_boundary(abs_end) {
                // Skip if next char is '.' followed by alpha — inserting a marker
                // here would create a Typst field access on the marker's return value.
                // e.g., "plugins.gradle.org" → "plugins#index[plugins].gradle.org" is broken.
                let next_is_field_access = source.as_bytes().get(abs_end) == Some(&b'.')
                    && source
                        .as_bytes()
                        .get(abs_end + 1)
                        .is_some_and(|b| b.is_ascii_alphabetic());
                if !next_is_field_access {
                    return Some(abs_end);
                }
            }
        }
        // Advance start to next char boundary in the lowered buffer.
        start = abs_pos + 1;
        while start < lower_source.len() && !lower_source.is_char_boundary(start) {
            start += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_ranges_skip_labels() {
        let source = "= Intro <intro>\n\nSome text about channels.";
        let ranges = collect_prose_ranges(source);
        // "channels" in prose should be in ranges
        let ch_pos = source.find("channels").unwrap();
        assert!(ranges.iter().any(|&(s, e)| ch_pos >= s && ch_pos + 8 <= e));
        // "<intro>" label should NOT be in ranges
        let label_pos = source.find("<intro>").unwrap();
        assert!(
            !ranges
                .iter()
                .any(|&(s, e)| label_pos >= s && label_pos + 7 <= e)
        );
    }

    #[test]
    fn prose_ranges_skip_link_targets() {
        let source = r#"See #link(<channels>)[Channels] for details."#;
        let ranges = collect_prose_ranges(source);
        // The label "<channels>" inside #link() should not be in ranges.
        // The #link call starts with Hash which is skipped entirely.
        let label_pos = source.find("<channels>").unwrap();
        assert!(
            !ranges
                .iter()
                .any(|&(s, e)| label_pos >= s && label_pos + 10 <= e),
            "label inside #link should be excluded from prose ranges"
        );
    }

    #[test]
    fn find_in_prose_skips_link_label() {
        let source = r#"See #link(<channels>)[Channels] for details about channels."#;
        let ranges = collect_prose_ranges(source);
        let offset = find_term_offset_in_prose(source, "channels", &ranges);
        assert!(offset.is_some(), "should find 'channels' in prose");
        // Should NOT match inside <channels> label
        let found_pos = offset.unwrap() - 8; // offset is AFTER the term
        let label_pos = source.find("<channels>").unwrap() + 1; // skip the <
        assert_ne!(found_pos, label_pos, "should not match inside label");
        // May match "Channels" in the [Channels] display text (valid prose)
        // or "channels" in the trailing prose — both are correct.
    }

    #[test]
    fn find_in_prose_skips_code() {
        let source = "Use `OAuth` for auth. OAuth is the standard.";
        let ranges = collect_prose_ranges(source);
        let offset = find_term_offset_in_prose(source, "OAuth", &ranges);
        assert!(offset.is_some());
        // Should match the prose occurrence, not inside backticks
        let found_pos = offset.unwrap() - 5;
        let code_pos = source.find("`OAuth`").unwrap() + 1;
        assert_ne!(found_pos, code_pos, "should not match inside code");
    }

    #[test]
    fn find_in_prose_multiword_across_nodes() {
        let source = "Use API key for access.";
        let ranges = collect_prose_ranges(source);
        let offset = find_term_offset_in_prose(source, "API key", &ranges);
        assert!(
            offset.is_some(),
            "multi-word term should match across text+space nodes"
        );
    }

    #[test]
    fn find_in_prose_skips_field_access_position() {
        // "plugins.gradle.org" — inserting after "plugins" would create
        // #index[plugins].gradle which Typst reads as field access.
        let source = "Visit plugins.gradle.org for plugins and extensions.";
        let ranges = collect_prose_ranges(source);
        let offset = find_term_offset_in_prose(source, "plugins", &ranges);
        assert!(offset.is_some(), "should find 'plugins' in safe position");
        let found_pos = offset.unwrap() - 7;
        // Should match the second "plugins" (not followed by '.g')
        let safe_pos = source.rfind("plugins").unwrap();
        assert_eq!(found_pos, safe_pos, "should match the safe occurrence");
    }

    #[test]
    fn find_in_prose_field_access_only_occurrence_skipped() {
        // If the only occurrence is followed by '.alpha', skip it entirely.
        let source = "Visit plugins.gradle.org for details.";
        let ranges = collect_prose_ranges(source);
        let offset = find_term_offset_in_prose(source, "plugins", &ranges);
        assert!(
            offset.is_none(),
            "should not match when only in dotted name"
        );
    }

    #[test]
    fn find_in_prose_not_found_only_in_syntax() {
        let source = r#"#set text(font: "channels")"#;
        let ranges = collect_prose_ranges(source);
        let offset = find_term_offset_in_prose(source, "channels", &ranges);
        assert!(
            offset.is_none(),
            "term only in function call should not match"
        );
    }
}
