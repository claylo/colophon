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
/// headings, and frontmatter while preserving paragraphs, list items,
/// and image alt text.
///
/// Headings are intentionally excluded — they tend to be short, generic
/// phrases ("Introduction", "Setup") that add noise to keyword extraction.
/// Any indexable term worth capturing will appear in the body prose.
pub fn extract_prose(markdown: &str) -> String {
    let content = strip_frontmatter(markdown);
    let parser = Parser::new_ext(content, Options::empty());

    let mut prose = String::with_capacity(content.len() / 2);
    let mut in_code_block = false;
    let mut in_heading = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
            }
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
            }
            Event::Text(text) if !in_code_block && !in_heading => {
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
            Event::End(TagEnd::Paragraph | TagEnd::Item) => {
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
    // Work with char indices to handle multi-byte UTF-8 correctly.
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();

    // Find the char offset of the match in the lowercased text.
    let lower_char_offset = lower_text
        .char_indices()
        .position(|(byte_pos, _)| lower_text[byte_pos..].starts_with(&lower_term))?;

    // Map char offset back to original text. Since to_lowercase() can change
    // byte lengths, we walk char-by-char through the original text.
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    if lower_char_offset >= char_indices.len() {
        return None;
    }

    let match_start_char = lower_char_offset;
    let term_char_len = term.chars().count();
    let match_end_char = (match_start_char + term_char_len).min(char_indices.len());

    // Window in chars, not bytes.
    let window_start_char = match_start_char.saturating_sub(window);
    let window_end_char = (match_end_char + window).min(char_indices.len());

    // Convert char positions to byte positions.
    let start_byte = char_indices[window_start_char].0;
    let end_byte = if window_end_char >= char_indices.len() {
        text.len()
    } else {
        char_indices[window_end_char].0
    };

    // Snap to word boundaries within the window.
    let snippet = &text[start_byte..end_byte];
    let trimmed = if window_start_char > 0 {
        // Skip to first whitespace, then past it
        snippet
            .find(char::is_whitespace)
            .and_then(|pos| snippet[pos..].find(|c: char| !c.is_whitespace()))
            .map_or(snippet, |skip| {
                &snippet[snippet.find(char::is_whitespace).unwrap() + skip..]
            })
    } else {
        snippet
    };

    Some(trimmed.trim().to_string())
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
        assert!(!result.contains("Just a heading"));
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
    fn headings_excluded_from_prose() {
        let md = "# Main Title\n\n## Subtitle\n\nBody text.";
        let result = extract_prose(md);
        assert!(!result.contains("Main Title"));
        assert!(!result.contains("Subtitle"));
        assert!(result.contains("Body text."));
    }

    #[test]
    fn multiple_paragraphs_separated() {
        let md = "First paragraph.\n\nSecond paragraph.";
        let result = extract_prose(md);
        assert!(result.contains("First paragraph."));
        assert!(result.contains("Second paragraph."));
    }

    #[test]
    fn context_extraction_multibyte_chars() {
        let text = "Use the → arrow and ← arrow for navigation in the CLI";
        let result = extract_context(text, "navigation", 10);
        assert!(result.is_some());
        let ctx = result.unwrap();
        assert!(ctx.contains("navigation"));
    }

    #[test]
    fn context_extraction_emoji() {
        let text = "The 🔥 hot take is that OAuth tokens should expire quickly";
        let result = extract_context(text, "OAuth", 15);
        assert!(result.is_some());
        let ctx = result.unwrap();
        assert!(ctx.contains("OAuth"));
    }
}
