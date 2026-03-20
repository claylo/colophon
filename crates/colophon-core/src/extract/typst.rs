//! Typst parsing and prose text extraction.
//!
//! Walks the `typst-syntax` AST to extract prose text, skipping
//! headings, code blocks, raw blocks, math equations, labels,
//! and function calls — same strategy as the markdown extractor.

use typst_syntax::{parse, SyntaxKind, SyntaxNode};

/// Extract prose text from a Typst document.
///
/// Walks the syntax tree and collects `Text` nodes while skipping:
/// - Headings (noisy short phrases, same rationale as markdown)
/// - Raw/code blocks (`` `code` `` and `` ```lang ... ``` ``)
/// - Math equations (`$...$`)
/// - Labels (`<label>`) and references (`@ref`)
/// - Code-mode expressions after `#` (function calls, imports, etc.)
///
/// Strong (`*bold*`) and emphasis (`_italic_`) text is preserved —
/// the prose inside is meaningful for keyword extraction.
pub fn extract_prose(source: &str) -> String {
    let root = parse(source);
    let mut prose = String::with_capacity(source.len() / 2);
    walk_node(&root, &mut prose, false);
    prose.trim().to_string()
}

/// Recursively walk a syntax node, appending prose text.
fn walk_node(node: &SyntaxNode, out: &mut String, in_heading: bool) {
    // Skip entirely — these contain no extractable prose.
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
            | SyntaxKind::Hash // Code-mode entry — skip function calls, imports, set/show rules.
    ) {
        return;
    }

    // Heading — set flag so child Text nodes are skipped.
    if node.kind() == SyntaxKind::Heading {
        for child in node.children() {
            walk_node(child, out, true);
        }
        return;
    }

    match node.kind() {
        // Text leaf — the prose we want (unless inside a heading).
        SyntaxKind::Text if !in_heading => {
            let text = node.text();
            if !text.is_empty() {
                if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }

        // Smart quotes — treat as punctuation prose.
        SyntaxKind::SmartQuote if !in_heading => {
            out.push('"');
        }

        // Paragraph/line breaks.
        SyntaxKind::Parbreak | SyntaxKind::Linebreak => {
            out.push('\n');
        }

        // Space — preserve word boundaries.
        SyntaxKind::Space => {
            if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                out.push(' ');
            }
        }

        // Escape sequences — try to decode the visible character.
        SyntaxKind::Escape if !in_heading => {
            let text = node.text();
            // Typst escapes: \#, \*, \_, etc. — just emit the char after \.
            if text.starts_with('\\') && text.len() > 1 {
                out.push_str(&text[1..]);
            }
        }

        // Everything else — recurse into children.
        _ => {
            for child in node.children() {
                walk_node(child, out, in_heading);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text() {
        let result = extract_prose("Hello world. This is a test.");
        assert_eq!(result, "Hello world. This is a test.");
    }

    #[test]
    fn headings_excluded() {
        let typ = "= Main Title\n\n== Subtitle\n\nBody text here.";
        let result = extract_prose(typ);
        assert!(!result.contains("Main Title"));
        assert!(!result.contains("Subtitle"));
        assert!(result.contains("Body text here."));
    }

    #[test]
    fn raw_inline_stripped() {
        let typ = "Use the `println!` macro for output.";
        let result = extract_prose(typ);
        assert!(result.contains("Use the"));
        assert!(result.contains("macro for output."));
        assert!(!result.contains("println!"));
    }

    #[test]
    fn raw_block_stripped() {
        let typ = "Before code.\n\n```rust\nfn main() {}\n```\n\nAfter code.";
        let result = extract_prose(typ);
        assert!(result.contains("Before code."));
        assert!(result.contains("After code."));
        assert!(!result.contains("fn main()"));
    }

    #[test]
    fn math_stripped() {
        let typ = "The formula $x^2 + y^2 = z^2$ is well known.";
        let result = extract_prose(typ);
        assert!(result.contains("The formula"));
        assert!(result.contains("is well known."));
        assert!(!result.contains("x^2"));
    }

    #[test]
    fn display_math_stripped() {
        let typ = "Consider:\n\n$ sum_(i=0)^n i = n(n+1)/2 $\n\nThis is a sum.";
        let result = extract_prose(typ);
        assert!(!result.contains("sum_"));
        assert!(result.contains("This is a sum."));
    }

    #[test]
    fn bold_and_italic_preserved() {
        let typ = "This is *bold text* and _italic text_ in a sentence.";
        let result = extract_prose(typ);
        assert!(result.contains("bold text"));
        assert!(result.contains("italic text"));
    }

    #[test]
    fn list_items_preserved() {
        let typ = "Shopping list:\n\n- Apples\n- Bananas\n- Cherries\n";
        let result = extract_prose(typ);
        assert!(result.contains("Apples"));
        assert!(result.contains("Bananas"));
        assert!(result.contains("Cherries"));
    }

    #[test]
    fn enum_items_preserved() {
        let typ = "Steps:\n\n+ First step\n+ Second step\n+ Third step\n";
        let result = extract_prose(typ);
        assert!(result.contains("First step"));
        assert!(result.contains("Second step"));
    }

    #[test]
    fn term_list_preserved() {
        let typ = "/ OAuth: An authorization protocol.\n/ TLS: Transport layer security.\n";
        let result = extract_prose(typ);
        assert!(result.contains("OAuth"));
        assert!(result.contains("authorization protocol"));
        assert!(result.contains("TLS"));
    }

    #[test]
    fn labels_stripped() {
        let typ = "= Introduction <intro>\n\nSome text about the topic.";
        let result = extract_prose(typ);
        assert!(!result.contains("<intro>"));
        assert!(!result.contains("intro"));
        assert!(result.contains("Some text about the topic."));
    }

    #[test]
    fn references_stripped() {
        let typ = "As discussed in @intro, the approach works.";
        let result = extract_prose(typ);
        assert!(!result.contains("@intro"));
        assert!(result.contains("the approach works."));
    }

    #[test]
    fn links_stripped() {
        let typ = "Visit https://example.com for details.";
        let result = extract_prose(typ);
        assert!(!result.contains("https://"));
        assert!(result.contains("Visit"));
        assert!(result.contains("for details."));
    }

    #[test]
    fn function_calls_stripped() {
        let typ = "#set text(size: 12pt)\n\nActual content here.\n\n#pagebreak()";
        let result = extract_prose(typ);
        assert!(!result.contains("set text"));
        assert!(!result.contains("pagebreak"));
        assert!(result.contains("Actual content here."));
    }

    #[test]
    fn content_blocks_in_functions_preserved() {
        let typ = "#block[This prose should be extracted.]\n\nMore text.";
        let result = extract_prose(typ);
        assert!(result.contains("This prose should be extracted."));
        assert!(result.contains("More text."));
    }

    #[test]
    fn comments_stripped() {
        let typ = "// This is a comment\nVisible text.\n/* block comment */\nMore text.";
        let result = extract_prose(typ);
        assert!(!result.contains("This is a comment"));
        assert!(!result.contains("block comment"));
        assert!(result.contains("Visible text."));
        assert!(result.contains("More text."));
    }

    #[test]
    fn empty_document() {
        assert_eq!(extract_prose(""), "");
    }

    #[test]
    fn escape_sequences() {
        let typ = r"Use \# for a literal hash and \* for an asterisk.";
        let result = extract_prose(typ);
        assert!(result.contains("#"));
        assert!(result.contains("*"));
    }

    #[test]
    fn multiple_paragraphs() {
        let typ = "First paragraph with some content.\n\nSecond paragraph with more.";
        let result = extract_prose(typ);
        assert!(result.contains("First paragraph"));
        assert!(result.contains("Second paragraph"));
    }

    #[test]
    fn realistic_document() {
        let typ = r#"#set document(title: "My Book")
#set text(font: "New Computer Modern")

= Chapter 1: Authentication <ch1>

OAuth provides delegated authorization. OAuth 2.0 is the
current standard for token-based access control.

== Password Hashing

Password hashing uses *bcrypt* or _argon2_ for secure
storage. The `hash()` function takes a plaintext input.

$H(p) = "hash"$

See @ch1 for more details.

// TODO: add more content
"#;
        let result = extract_prose(typ);
        // Prose preserved
        assert!(result.contains("OAuth provides delegated authorization."));
        assert!(result.contains("Password hashing uses"));
        assert!(result.contains("bcrypt"));
        assert!(result.contains("argon2"));
        // Headings excluded
        assert!(!result.contains("Chapter 1"));
        assert!(!result.contains("Password Hashing"));
        // Code/math stripped
        assert!(!result.contains("hash()"));
        assert!(!result.contains("H(p)"));
        // Function calls stripped
        assert!(!result.contains("set document"));
        assert!(!result.contains("set text"));
        // Labels/refs stripped
        assert!(!result.contains("<ch1>"));
        assert!(!result.contains("@ch1"));
        // Comments stripped
        assert!(!result.contains("TODO"));
    }
}
