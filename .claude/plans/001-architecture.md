# Colophon Architecture Plan

> **Tagline**: Generate all the parts of your book you were going to skip.
>
> *Because your book deserves a rear end worth looking at.* 🍑

## What Colophon Does

Colophon scans a directory of Markdown files and generates:

1. **Back-of-book index** — terms → page/file locations
2. **Glossary** — terms → definitions

Both outputs come from a single extraction + curation pipeline. The user
points colophon at a directory, it does the NLP locally, sends candidates to
Claude for intelligent curation, and outputs annotated files or standalone
index/glossary documents ready for Typst (or other formats).

## Pipeline Overview

```
docs/*.md
    │
    ▼
┌──────────┐    local, pure Rust
│ EXTRACT  │    pulldown-cmark → keyword_extraction (YAKE + TF-IDF)
└────┬─────┘
     │  candidates.yaml (raw scored terms + context snippets)
     ▼
┌──────────┐    Anthropic API call
│  CURATE  │    Claude merges synonyms, builds hierarchy, generates
└────┬─────┘    definitions, kills junk, finds missed terms
     │  colophon-terms.yaml (curated term database)
     ▼
┌──────────┐    pure Rust, pluggable formatters
│  RENDER  │    annotate source files or emit standalone index/glossary
└──────────┘
     │
     ▼
  typst / latex / markdown / json
```

## CLI Commands

### `colophon init`
Creates a `.colophon.yaml` config in the current directory with sensible
defaults. Interactive prompts for source dir, output format, API key location.

### `colophon extract [--dir <path>]`
- Walks the directory, collects all `.md` files
- Strips frontmatter, code blocks, URLs, image refs (keep alt text)
- Runs YAKE per-document for single-doc importance
- Runs TF-IDF across the full corpus for cross-document weighting
- Merges candidates, deduplicates, scores
- Writes `colophon-candidates.yaml` with raw candidates + context

### `colophon curate [--candidates <file>]`
- Reads candidates file
- Sends to Claude API with a structured prompt:
  - Remove noise terms ("document", "example", "section")
  - Merge synonyms ("REST API" / "RESTful API" → one entry)
  - Build hierarchy ("OAuth" under "authentication")
  - Generate short glossary definitions for each term
  - Flag "main" vs "mention" entries (for bold page numbers)
  - Identify important terms YAKE missed
- Writes `colophon-terms.yaml` (the curated term database)

### `colophon render [--format typst] [--output <dir>]`
- Reads the curated term database
- Depending on mode:
  - `--annotate`: Inserts markers in copies of source `.md` files
  - `--standalone`: Emits separate index and/or glossary documents
- Format-specific output:
  - **typst**: `#index[term]` markers (for in-dexter), glossary as definition list
  - **latex**: `\index{term}` and `\gls{term}` markers
  - **markdown**: configurable syntax (e.g., `{term}`, footnotes)
  - **json**: raw structured data for custom consumers

### `colophon generate [--dir <path>] [--format typst]`
Convenience command: runs extract → curate → render in one shot.

## Term Database Schema (colophon-terms.yaml)

```yaml
# colophon-terms.yaml
version: 1
generated: 2026-03-09T12:00:00Z
source_dir: docs/

terms:
  oauth:
    display: "OAuth"
    definition: "An open standard for token-based authorization delegating access without sharing credentials."
    parent: authentication     # hierarchy: renders as "authentication > OAuth" in index
    aliases:                   # synonyms that map to this entry
      - "OAuth 2.0"
      - "OAuth2"
    locations:
      - file: "03-auth.md"
        main: true             # bold page number in index
        context: "OAuth provides delegated authorization..."
      - file: "07-api.md"
        main: false
        context: "The API requires OAuth tokens for..."
      - file: "12-security.md"
        main: false
    tags: []                   # future: user-defined categories

  authentication:
    display: "Authentication"
    definition: "The process of verifying the identity of a user or system."
    children:                  # populated automatically from parent refs
      - oauth
      - jwt
      - api_keys
    locations:
      - file: "03-auth.md"
        main: true
```

## Crate Organization

### colophon-core (fat core)

```
crates/colophon-core/src/
├── lib.rs
├── config.rs          # existing — extend with extraction/curation config
├── error.rs           # existing — extend with new error variants
├── extract/
│   ├── mod.rs         # extraction pipeline orchestrator
│   ├── markdown.rs    # markdown parsing, text extraction (pulldown-cmark)
│   ├── keywords.rs    # YAKE + TF-IDF wrapper (keyword_extraction crate)
│   └── candidates.rs  # candidate types, scoring, serialization
├── curate/
│   ├── mod.rs         # curation pipeline orchestrator
│   ├── claude.rs      # Anthropic API client, prompt engineering
│   └── terms.rs       # curated term database types + serialization
└── render/
    ├── mod.rs         # renderer trait + dispatch
    ├── typst.rs       # Typst output (in-dexter markers, glossary)
    ├── latex.rs       # LaTeX output (\index, \gls)
    ├── markdown.rs    # Markdown output (configurable markers)
    └── json.rs        # JSON export
```

### colophon (thin CLI)

```
crates/colophon/src/
├── main.rs            # existing — add new command dispatch
├── lib.rs             # existing — add new Commands variants
├── commands/
│   ├── mod.rs         # existing
│   ├── doctor.rs      # existing
│   ├── info.rs        # existing
│   ├── init.rs        # new: config scaffolding
│   ├── extract.rs     # new: extraction command
│   ├── curate.rs      # new: curation command
│   ├── render.rs      # new: rendering command
│   └── generate.rs    # new: all-in-one convenience
└── observability.rs   # existing
```

## New Dependencies (colophon-core)

```toml
# Markdown parsing
pulldown-cmark = "0.12"

# Keyword extraction (pure Rust: YAKE, TF-IDF, TextRank, RAKE)
keyword_extraction = { version = "1.5", features = ["yake", "tf_idf"] }
stop-words = "0.8"

# YAML serialization for term database
serde_yaml = "0.9"

# HTTP client for Anthropic API (blocking — no async runtime needed)
ureq = { version = "3", features = ["json", "rustls"] }

# Glob/walk for file discovery
globset = "0.4"
walkdir = "2"
```

## Configuration (extend existing Config)

```yaml
# .colophon.yaml
log_level: info

# Source configuration
source:
  dir: docs/               # directory to scan
  extensions: [md]         # file extensions to process
  exclude: [README.md]     # files to skip

# Extraction tuning
extract:
  algorithms: [yake, tf_idf]
  ngram_range: [1, 3]      # extract 1-word to 3-word phrases
  min_score: 0.1           # minimum relevance threshold
  max_candidates: 500      # cap before sending to curation

# Claude curation
curate:
  provider: anthropic
  model: claude-sonnet-4-20250514
  # API key from env: ANTHROPIC_API_KEY
  # or from config:
  # api_key_cmd: "op read 'op://Dev/Anthropic API/credential'"
  max_terms: 200           # max terms in final output
  generate_definitions: true

# Output
render:
  format: typst            # typst | latex | markdown | json
  outputs: [index, glossary]
  annotate_sources: false  # if true, write annotated copies of source files
  output_dir: build/       # where to write rendered output
```

## Implementation Phases

### Phase 1: Extract
- [ ] Add `pulldown-cmark`, `keyword_extraction`, `stop-words`, `walkdir` deps
- [ ] Implement `extract::markdown` — parse MD, strip non-prose, emit clean text
- [ ] Implement `extract::keywords` — YAKE + TF-IDF wrapper
- [ ] Implement `extract::candidates` — types, scoring, YAML serialization
- [ ] Implement `extract::mod` — orchestrate the pipeline
- [ ] Wire up `colophon extract` command
- [ ] Tests: known markdown corpus → expected candidates

### Phase 2: Curate
- [ ] Add `ureq` dep
- [ ] Implement `curate::claude` — API client, prompt construction
- [ ] Implement `curate::terms` — curated term database types
- [ ] Wire up `colophon curate` command
- [ ] Tests: mock API responses → expected term database

### Phase 3: Render
- [ ] Implement `render` trait and dispatch
- [ ] Implement `render::typst` — in-dexter markers + glossary
- [ ] Implement `render::json` — structured export
- [ ] Wire up `colophon render` command
- [ ] Tests: known term database → expected output

### Phase 4: Polish
- [ ] `colophon init` command
- [ ] `colophon generate` (all-in-one)
- [ ] Add `render::latex` and `render::markdown` formatters
- [ ] README rewrite with actual usage examples
- [ ] `--dry-run` support for all commands
- [ ] MCP server wrapper (future — make colophon available as a tool)

## Design Decisions

1. **YAML for term database, not TOML.** Nested structures (terms with
   children, locations with context) are painful in TOML. YAML handles
   hierarchy naturally.

2. **YAKE over RAKE.** YAKE is unsupervised and corpus-independent — it
   works on single documents without needing a reference collection. RAKE
   has trouble with multi-word phrases.

3. **TF-IDF as cross-document signal.** YAKE finds what's important *in*
   a document; TF-IDF finds what's important *about* a document relative
   to the corpus. Both signals together are stronger than either alone.

4. **Claude for curation, not extraction.** The NLP extraction runs locally
   and is fast/free. Claude's value is in the *judgment* calls: synonym
   merging, hierarchy building, definition generation, junk removal. This
   keeps API costs minimal (one call, not 50).

5. **Pluggable renderers via trait.** Adding a new output format should be
   one file implementing a trait, not a rewrite.

6. **Term database as the artifact.** The YAML term database is the product.
   Renderers are convenience. Users who need a format we don't support can
   consume the YAML directly.
