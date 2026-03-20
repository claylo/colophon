# colophon

[![CI](https://github.com/claylo/colophon/actions/workflows/ci.yml/badge.svg)](https://github.com/claylo/colophon/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/colophon.svg)](https://crates.io/crates/colophon)
[![docs.rs](https://docs.rs/colophon/badge.svg)](https://docs.rs/colophon)
[![MSRV](https://img.shields.io/badge/MSRV-1.89.0-blue.svg)](https://github.com/claylo/colophon)

Generate back-of-book indexes and glossaries from Markdown or Typst source files. Three commands take you from raw manuscript to print-ready index:

```
colophon extract --dir ./chapters
colophon curate
colophon render --terms colophon-terms.yaml --glossary -o ./output
```

**Extract** scans your documents for keyword candidates using YAKE and TF-IDF. **Curate** sends those candidates to Claude for professional indexing judgment — merging synonyms, building hierarchy, writing definitions. **Render** inserts index markers and generates a glossary in your source format.

## How It Works

```
  ┌─────────┐      ┌────────┐      ┌────────┐
  │ extract │ ──── │ curate │ ──── │ render │
  └─────────┘      └────────┘      └────────┘
    .md/.typ    candidates.yaml   terms.yaml    annotated files
    ────────>   ────────────────> ──────────>   + glossary.typ
```

1. **Extract** walks your source directory, parses Markdown (pulldown-cmark) and Typst (typst-syntax AST), runs YAKE keyword extraction per document, then TF-IDF across the corpus. Outputs `colophon-candidates.yaml`.

2. **Curate** sends candidates to Claude via the `claude` CLI. Claude acts as a professional book indexer: keeps terms worth looking up, kills noise, merges synonyms into canonical entries with aliases, builds parent/child hierarchy, writes glossary definitions, and identifies which files discuss each term substantively. Outputs `colophon-terms.yaml`.

3. **Render** reads the curated terms and inserts format-specific index markers into your source files. For Typst, it generates `#index[term]` and `#index-main[term]` markers (compatible with the [in-dexter](https://typst.app/universe/package/in-dexter/) package) and a `glossary.typ` with linked cross-references.

### Incremental Curate

After your first full curate run, subsequent runs are incremental by default. Colophon diffs fresh candidates against your existing `colophon-terms.yaml`:

- **No new terms?** Mechanical location refresh only. Zero cost, sub-second.
- **New terms found?** Sends only the new candidates to Claude with a compact summary of the existing index. Typical cost: ~$0.50 vs ~$8 for a full rebuild.

Force a full rebuild any time with `--full-rebuild`.

## Installation

### From Source

```bash
cargo install colophon
```

### Homebrew (macOS and Linux)

```bash
brew install claylo/brew/colophon
```

### Pre-built Binaries

Download from the [releases page](https://github.com/claylo/colophon/releases). Binaries are available for macOS (Apple Silicon / Intel), Linux (x86_64 / ARM64), and Windows.

### Requirements

- The `curate` command requires the [Claude CLI](https://claude.com/claude-code) (`claude`) in your PATH.
- Typst rendering uses [in-dexter](https://typst.app/universe/package/in-dexter/) markers. Add `#import "@preview/in-dexter:0.6.1": *` to your document.

## Quick Start

Create a config file in your project root:

```yaml
# .colophon.yaml
source:
  dir: ./chapters
  extensions: [typ]
```

Run the pipeline:

```bash
# 1. Extract keyword candidates
colophon extract

# 2. Curate with Claude (estimate cost first)
colophon curate --dry-run
colophon curate

# 3. Render index markers and glossary
colophon render --glossary -o ./output
```

After editing your manuscript, run it again. The curate step auto-detects the existing terms file and runs incrementally:

```bash
colophon extract
colophon curate        # incremental — only new terms sent to Claude
colophon render --glossary -o ./output
```

## Commands

### extract

Scan source files and produce keyword candidates:

```bash
colophon extract --dir ./chapters -o candidates.yaml
```

| Flag | Default | Description |
|------|---------|-------------|
| `-d, --dir` | config `source.dir` | Source directory to scan |
| `-o, --output` | `colophon-candidates.yaml` | Output file |
| `--json` | | Output as JSON instead of YAML |

Extraction parameters (n-gram range, min score, max candidates, stop words, exclude terms, known terms) are configured in your config file. See `config/colophon.yaml.example` for all options.

### curate

Send candidates to Claude for professional indexing:

```bash
# Estimate cost without invoking Claude
colophon curate --dry-run

# Run with Opus for best quality
colophon curate -m opus

# Set a budget ceiling
colophon curate --max-budget-usd 10
```

| Flag | Default | Description |
|------|---------|-------------|
| `--candidates` | `colophon-candidates.yaml` | Path to candidates file |
| `-m, --model` | config `curate.model` | Claude model (`sonnet`, `opus`, `haiku`) |
| `--dry-run` | | Estimate cost and exit |
| `--max-budget-usd` | | Abort if estimated cost exceeds this |
| `--full-rebuild` | | Force full rebuild (skip incremental) |
| `--full` | | Send full YAML with context snippets |
| `-o, --output-dir` | `.` | Where to write `colophon-terms.yaml` |

**Incremental mode** activates automatically when `colophon-terms.yaml` exists in the output directory. It re-extracts candidates locally, diffs against existing terms, and sends only new candidates to Claude. Use `--full-rebuild` to bypass this.

Pass extra flags to the `claude` CLI after `--`:

```bash
colophon curate -- --max-turns 3
```

### render

Insert index markers and generate a glossary:

```bash
# Annotate source files with index markers
colophon render --terms colophon-terms.yaml -o ./output

# Also generate a glossary
colophon render --terms colophon-terms.yaml --glossary -o ./output

# Only mark substantive discussions (bold page numbers)
colophon render --terms colophon-terms.yaml --main-only -o ./output
```

| Flag | Default | Description |
|------|---------|-------------|
| `--terms` | `colophon-terms.yaml` | Curated terms file |
| `-d, --dir` | from terms file | Source directory |
| `-o, --output-dir` | `.` | Where to write annotated files |
| `--format` | `typst` | Output format |
| `--glossary` | | Also emit `glossary.typ` |
| `--main-only` | | Only mark `main: true` locations |
| `--glossary-spacing` | | Gap between entries (e.g., `12pt`) |

For Typst output, render produces:
- `#index[term]` markers at term locations
- `#index-main[term]` for substantive discussions
- `#index("parent", "child")` for hierarchical entries
- A `glossary.typ` with code-mode `#terms()` entries, label anchors, and linked cross-references

## Configuration

Config files are discovered automatically (highest precedence first):

1. `.colophon.<ext>` in current directory or any parent
2. `colophon.<ext>` in current directory or any parent
3. `~/.config/colophon/config.<ext>` (user config)

Supported formats: TOML, YAML, JSON. Search stops at `.git` boundaries.

See [`config/colophon.yaml.example`](config/colophon.yaml.example) for all available options with documentation.

### Minimal Config

```yaml
source:
  dir: ./chapters
  extensions: [typ]

curate:
  model: opus
  max_budget_usd: 10.0
```

## Global Options

Every command accepts these flags:

| Flag | Description |
|------|-------------|
| `-v, --verbose` | More detail (repeatable: `-vv` for trace) |
| `-q, --quiet` | Only print errors |
| `--json` | Machine-readable JSON output |
| `-C, --chdir` | Run as if started in a different directory |
| `-c, --config` | Explicit config file path |

## Development

```
crates/
  colophon/         # CLI binary (thin shell)
  colophon-core/    # Shared library (extract, curate, render)
  xtask/            # Dev automation
```

Prerequisites: Rust 1.89.0+, [just](https://github.com/casey/just), [cargo-nextest](https://nexte.st/)

```bash
just check          # fmt + clippy + deny + test + doc-test
just test           # cargo nextest run
just clippy         # lint
```

See [AGENTS.md](AGENTS.md) for development conventions.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
