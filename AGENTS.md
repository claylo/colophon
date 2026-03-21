# colophon


## Workspace Layout

This is a Cargo workspace. All crates live under `crates/`:

| Crate | Purpose |
|-------|---------|
| `colophon` | CLI binary |
| `colophon-core` | Shared library (config, types, logic) |
| `xtask` | Dev automation (completions, man pages) |

Configuration files live in `config/` with `.toml.example` and `.yaml.example` templates.
## Commands

Use `just` for all dev tasks:

```
just check          # fmt + clippy + deny + test + doc-test (run before pushing)
just test           # cargo nextest run
just clippy         # lint with pinned toolchain
just fmt            # cargo fmt --all
just deny           # security/license audit
just fix            # auto-fix clippy warnings
just release-check  # pre-release validation
just outdated       # check for outdated dependencies
just upgrade        # update deps in Cargo.toml and Cargo.lock
```


**Tests use `cargo nextest run`**, not `cargo test`. Doc tests are separate: `cargo test --doc`.

## Rust Conventions

- **Edition 2024**, MSRV **1.89.0**, toolchain pinned in `rust-toolchain.toml`
- `unsafe_code = "deny"` — no unsafe unless explicitly allowed with a `// SAFETY:` comment
- Clippy `all` = warn, `nursery` = warn — treat warnings as errors in CI
- Use `anyhow::Result` in the binary, `thiserror` for library error types
- Shared logic belongs in `colophon-core`; the CLI crate handles argument parsing and I/O


**IMPORTANT — THIN CLI, FAT CORE.** Feature logic and new dependencies belong in `colophon-core`, not `colophon`. The CLI crate is a thin shell: argument parsing, I/O, and wiring. This keeps the core testable without subprocess gymnastics and leaves the door open for other frontends (WASM, Tauri, etc).
## Adding CLI Commands

1. Create `crates/colophon/src/commands/your_cmd.rs`
2. Add the variant to `Commands` enum in `crates/colophon/src/lib.rs`
3. Wire it up in `match cli.command` in `main.rs`
4. Add integration tests in `crates/colophon/tests/`

## Do Not

- Commit anything in `target/`
- Add dependencies without checking `deny.toml` license policy (`just deny`)
- Skip `--all-targets --all-features` when running clippy
- Use `cargo test` instead of `cargo nextest run`
- Run raw cargo commands when a `just` recipe exists
