//! Markdown parsing and prose text extraction.
//!
//! Strips frontmatter, code blocks, URLs, and image refs while
//! preserving alt text and prose content for keyword analysis.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Strip YAML frontmatter delimited by `---` lines.
///
/// If the content starts with `---` (after optional leading whitespace),
/// everything up to and including the closing `---` line is removed.
/// If no closing delimiter is found, the content is returned as-is.
fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    // Skip the opening `---` line
    let after_open = match trimmed.strip_prefix("---") {
        Some(rest) => {
            // Must be followed by newline or be at end
            if let Some(after_newline) = rest.strip_prefix('\n') {
                after_newline
            } else if let Some(after_newline) = rest.strip_prefix("\r\n") {
                after_newline
            } else if rest.is_empty() {
                return content; // Just "---" with nothing after: no closing delimiter
            } else {
                return content; // "---" followed by non-whitespace chars on same line
            }
        }
        None => return content,
    };

    // Find closing `---` on its own line
    for (i, line) in after_open.lines().enumerate() {
        if line.trim() == "---" {
            // Find the byte offset right after the closing ---\n
            let closing_start = after_open.as_ptr() as usize - content.as_ptr() as usize;
            let line_start_in_after = after_open
                .lines()
                .take(i)
                .fold(0usize, |acc, l| acc + l.len() + 1); // +1 for \n
            let line_end = line_start_in_after + line.len();
            let past_closing = closing_start + line_end;

            // Skip past the newline after closing ---
            if past_closing < content.len() {
                let rest = &content[past_closing..];
                if let Some(stripped) = rest.strip_prefix('\n') {
                    return stripped;
                } else if let Some(stripped) = rest.strip_prefix("\r\n") {
                    return stripped;
                }
                return rest;
            }
            return "";
        }
    }

    // No closing delimiter found
    content
}

/// Extract prose text from markdown, stripping code blocks, inline code,
/// and frontmatter while preserving headings, paragraphs, list items,
/// and image alt text.
pub fn extract_prose(markdown: &str) -> String {
    let content = strip_frontmatter(markdown);
    let parser = Parser::new_ext(content, Options::empty());

    let mut prose = String::with_capacity(content.len() / 2);
    let mut in_code_block = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
            }
            Event::Text(text) if !in_code_block => {
                if !prose.is_empty() && !prose.ends_with('\n') && !prose.ends_with(' ') {
                    prose.push(' ');
                }
                prose.push_str(&text);
            }
            Event::Code(_) => {
                // Skip inline code
            }
            Event::SoftBreak | Event::HardBreak => {
                prose.push('\n');
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                prose.push('\n');
            }
            _ => {}
        }
    }

    prose.trim().to_string()
}

/// Extract a context snippet around the first occurrence of `term` in `text`.
///
/// Performs a case-insensitive search. If found, returns a substring of up to
/// `window` characters on each side of the match, snapped to word boundaries.
/// Returns `None` if the term is not found.
pub fn extract_context(text: &str, term: &str, window: usize) -> Option<String> {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();

    let match_start = lower_text.find(&lower_term)?;
    let match_end = match_start + term.len();

    // Determine raw window bounds
    let raw_start = match_start.saturating_sub(window);
    let raw_end = (match_end + window).min(text.len());

    // Snap start to word boundary (move forward to first non-space after a space,
    // or stay at 0 if we're at the beginning)
    let start = if raw_start == 0 {
        0
    } else {
        // Find the next space from raw_start, then skip past it
        text[raw_start..match_start]
            .find(' ')
            .map_or(raw_start, |pos| raw_start + pos + 1)
    };

    // Snap end to word boundary (find last space before raw_end)
    let end = if raw_end >= text.len() {
        text.len()
    } else {
        text[match_end..raw_end]
            .rfind(' ')
            .map_or(raw_end, |pos| match_end + pos)
    };

    Some(text[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_prose_extraction() {
        let md = "Hello world. This is a test.";
        let result = extract_prose(md);
        assert_eq!(result, "Hello world. This is a test.");
    }

    #[test]
    fn code_block_stripping() {
        let md = "Before code.\n\n```rust\nfn main() {}\n```\n\nAfter code.";
        let result = extract_prose(md);
        assert!(result.contains("Before code."));
        assert!(result.contains("After code."));
        assert!(!result.contains("fn main()"));
    }

    #[test]
    fn inline_code_stripping() {
        let md = "Use the `println!` macro for output.";
        let result = extract_prose(md);
        assert!(result.contains("Use the"));
        assert!(result.contains("macro for output."));
        assert!(!result.contains("println!"));
    }

    #[test]
    fn frontmatter_stripping() {
        let md = "---\ntitle: Test\ndate: 2026-01-01\n---\n\nActual content here.";
        let result = extract_prose(md);
        assert_eq!(result, "Actual content here.");
        assert!(!result.contains("title"));
    }

    #[test]
    fn no_frontmatter_handling() {
        let md = "# Just a heading\n\nSome paragraph text.";
        let result = extract_prose(md);
        assert!(result.contains("Just a heading"));
        assert!(result.contains("Some paragraph text."));
    }

    #[test]
    fn alt_text_preservation() {
        let md = "Look at this: ![a cute cat](cat.png) in the text.";
        let result = extract_prose(md);
        assert!(result.contains("a cute cat"));
    }

    #[test]
    fn list_items_preservation() {
        let md = "Shopping list:\n\n- Apples\n- Bananas\n- Cherries\n";
        let result = extract_prose(md);
        assert!(result.contains("Apples"));
        assert!(result.contains("Bananas"));
        assert!(result.contains("Cherries"));
    }

    #[test]
    fn context_extraction_found() {
        let text = "The quick brown fox jumps over the lazy dog near the river";
        let result = extract_context(text, "fox", 15);
        assert!(result.is_some());
        let ctx = result.unwrap();
        assert!(ctx.contains("fox"));
    }

    #[test]
    fn context_extraction_not_found() {
        let text = "The quick brown fox jumps over the lazy dog";
        let result = extract_context(text, "elephant", 10);
        assert!(result.is_none());
    }

    #[test]
    fn context_extraction_case_insensitive() {
        let text = "The Quick Brown Fox jumps over the lazy dog";
        let result = extract_context(text, "quick brown", 10);
        assert!(result.is_some());
        let ctx = result.unwrap();
        // Should contain the original-case text
        assert!(ctx.contains("Quick Brown"));
    }

    #[test]
    fn empty_document() {
        let result = extract_prose("");
        assert_eq!(result, "");
    }

    #[test]
    fn only_frontmatter_document() {
        let md = "---\ntitle: Nothing else\nauthor: Nobody\n---\n";
        let result = extract_prose(md);
        assert_eq!(result, "");
    }

    #[test]
    fn strip_frontmatter_no_closing() {
        let content = "---\ntitle: broken\nno closing delimiter";
        let result = strip_frontmatter(content);
        assert_eq!(result, content);
    }

    #[test]
    fn heading_produces_text() {
        let md = "# Main Title\n\n## Subtitle\n\nBody text.";
        let result = extract_prose(md);
        assert!(result.contains("Main Title"));
        assert!(result.contains("Subtitle"));
        assert!(result.contains("Body text."));
    }

    #[test]
    fn multiple_paragraphs_separated() {
        let md = "First paragraph.\n\nSecond paragraph.";
        let result = extract_prose(md);
        assert!(result.contains("First paragraph."));
        assert!(result.contains("Second paragraph."));
    }
}
