# Phase 2: Curate

**Date:** 2026-03-10
**Status:** Design approved, ready for implementation

## Summary

`colophon curate` reads raw candidates from Phase 1, shells out to the
`claude` CLI with a JSON Schema constraint, and writes a curated term
database. No new dependencies — uses the user's authenticated Claude
subscription via `std::process::Command`.

## Data Flow

```
colophon-candidates.yaml
        │
        ▼
  ┌─────────────┐
  │ curate::run │  read candidates, build prompt, shell out
  └──────┬──────┘
         │  claude -p --json-schema ... --output-format json
         ▼
  ┌─────────────┐
  │  claude CLI │  user's authenticated subscription
  └──────┬──────┘
         │  { "structured_output": { "terms": [...], "suggested": [...] } }
         ▼
  ┌─────────────┐
  │ post-process│  map locations, invert hierarchy, flag main refs
  └──────┬──────┘
         │
         ▼
  colophon-terms.yaml
```

## Config (`CurateConfig`)

```yaml
curate:
  model: sonnet                              # claude --model value
  max_terms: 200                             # cap on curated output
  candidates: colophon-candidates.yaml       # input file
  system_prompt: ~                           # replaces built-in system prompt
  prompt: ~                                  # appended after candidates as user guidance
```

- **`system_prompt`** — replaces the built-in system prompt entirely.
  The default handles structural instructions: what curation means, how
  to use the schema, what to keep/kill/merge. Leave null to use it.
- **`prompt`** — appended after the candidates payload. Domain-specific
  steering ("this is a book about Claude Code, merge Bedrock/Vertex
  variants..."). Most users only set this.

## CLI

```
colophon curate [--candidates <file>] [--output <file>] [--model <model>] [-- <claude-args>...]
```

- `--candidates` overrides config `curate.candidates`
- `--output` defaults to `colophon-terms.yaml`
- `--model` overrides config `curate.model`
- `--json` emits JSON instead of YAML
- Everything after `--` is passed through to `claude` CLI
  (e.g., `-- --max-budget-usd 5.00 --verbose`)

## Crate Layout

```
crates/colophon-core/src/curate/
├── mod.rs       # pipeline orchestrator: run()
├── claude.rs    # shell out to claude CLI, parse response
└── terms.rs     # CuratedTermsFile, CuratedTerm, serialization
```

## Types

### CuratedTerm

| Field | Type | Source |
|---|---|---|
| `term` | `String` | Claude output |
| `definition` | `String` | Claude output |
| `parent` | `Option<String>` | Claude output |
| `aliases` | `Vec<String>` | Claude output |
| `see_also` | `Vec<String>` | Claude output |
| `children` | `Vec<String>` | Post-processing (invert parent refs) |
| `locations` | `Vec<TermLocation>` | Post-processing (map from candidates) |

### TermLocation

| Field | Type | Source |
|---|---|---|
| `file` | `String` | Candidates file |
| `main` | `bool` | Claude output (`main_files`) |
| `context` | `String` | Candidates file |

### CuratedTermsFile

| Field | Type |
|---|---|
| `version` | `u32` |
| `generated` | `String` |
| `source_dir` | `String` |
| `document_count` | `usize` |
| `terms` | `Vec<CuratedTerm>` |

## Schema

The JSON Schema lives in `config/curate-schema.yaml` (authored in YAML,
converted to JSON at request time via `yj`). Claude outputs:

- **`terms[]`** — curated entries with `term`, `definition`, `parent`,
  `aliases`, `see_also`, `main_files`
- **`suggested[]`** — terms Claude spotted that YAKE missed, with
  `term`, `definition`, `parent`

Location data is NOT in Claude's output. Post-processing maps it from
the candidates file using `term` + `aliases` as join keys.

## Pipeline Steps (`curate::run()`)

1. **Read candidates** — deserialize `CandidatesFile` from YAML
2. **Find claude** — verify `claude` is in PATH and authenticated
3. **Build prompt** — system prompt (default or override) + candidates
   YAML + optional user prompt
4. **Convert schema** — read `curate-schema.yaml`, convert via `yj`
   (or embed pre-converted JSON as a const)
5. **Shell out** — `Command::new("claude")` with:
   - `-p` (print mode)
   - `--model {model}`
   - `--system-prompt {system_prompt}`
   - `--output-format json`
   - `--json-schema {schema_json}`
   - `--max-turns 1`
   - any passthrough `claude_args`
   - prompt piped via stdin
6. **Parse response** — deserialize JSON, extract `structured_output`
7. **Post-process:**
   - Map locations: for each curated term, look up `term` + `aliases`
     in candidates, merge their locations
   - Flag main: `main_files` from Claude → `main: true` on matching
     locations
   - Invert hierarchy: scan `parent` fields, populate `children`
   - Validate: warn on dangling parent refs, duplicate aliases
8. **Write** — serialize `CuratedTermsFile` to YAML

## Error Types

| Variant | Cause |
|---|---|
| `ClaudeNotFound` | `claude` not in PATH |
| `ClaudeAuthFailed` | Not logged in (`claude auth status` check) |
| `ClaudeFailed { exit_code, stderr }` | Non-zero exit |
| `ParseResponse` | JSON parse or schema mismatch |
| `NoCandidates` | Candidates file missing or empty |
| `SchemaConversion` | `yj` not found or conversion failed |
| `Serialize` | Output serialization failure |

## Shell Invocation Detail

```bash
cat <<'PROMPT' | claude -p \
  --model sonnet \
  --system-prompt "$(cat system-prompt.txt)" \
  --output-format json \
  --json-schema "$(yj < curate-schema.yaml)" \
  --max-turns 1 \
  {claude_args...}

<candidates>
{candidates_yaml}
</candidates>

{user_prompt}
PROMPT
```

## Testing Strategy

- **Unit tests (no claude):** mock Claude response JSON → verify
  post-processing (location mapping, hierarchy inversion, main flagging,
  dangling parent warnings)
- **Schema tests:** round-trip `curate-schema.yaml` through serde,
  verify it parses as valid JSON Schema
- **Integration test:** small candidates file + mock `claude` shell
  script that returns canned JSON → verify full pipeline

## Not In Scope (Phase 2.1+)

- Direct Anthropic API fallback (`ureq`)
- `suggested` term location search in source files
- Cost tracking beyond what `--max-budget-usd` provides
- Caching / incremental re-curation
- `yj` elimination (embed schema as compiled-in JSON const)

## Implementation Order

1. `CurateConfig` in `config.rs` + config file examples
2. `curate::terms` — types + serialization + unit tests
3. `curate::claude` — shell out + parse response + unit tests with mock
4. `curate::mod` — pipeline orchestrator with post-processing
5. `commands/curate.rs` — CLI wiring
6. Default system prompt (embedded const string)
7. Integration test with mock claude script
