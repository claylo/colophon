# Handoff: Pre-Release Polish

**Date:** 2026-03-26
**Branch:** `main`
**State:** Green — all tests pass, clippy clean. Uncommitted: banner + compact help changes in `commit.txt`.

## Where things stand

Two known issues from the release-ready handoff are fixed and merged (PR #18): dead `_candidates_yaml` param removed from `run_incremental()`, and curate prompt tightened to reduce phantom locations. CLI now displays an ASCII banner (Calvin S font, bold slate `#7b8699`) on extract/curate steps, and `--help` renders in compact single-line format. Release prep artifacts are in `scratch/` and `dist/`.

## Decisions made

- **Calvin S figlet font** for ASCII banner — box-drawing characters fit the book-production theme.
- **Bold slate `#7b8699` truecolor** for banner — matches claylo brand palette. Suppressed for `--json` and non-terminal stderr via `IsTerminal` check.
- **Compact help via `HelpShort`** — `disable_help_flag = true` + custom `--help` with `clap::ArgAction::HelpShort` makes both `-h` and `--help` render the same tight format.
- **Prompt tuning over code fix** for phantom locations — added point 8 (alias source text variants) and tightened point 7 (main_files verbatim only). Custom `system_prompt` in config bypasses defaults, so no impact on override users.

## What's next

1. **Commit the banner/help changes** — `commit.txt` is ready, run `gtxt` on a feature branch.
2. **Add `--help-llm`** — comprehensive flat markdown dump (pipeline, config, workflows, all flags). Clay is shaping the content. Implementation: `include_str!` a markdown file, print to stdout.
3. **Cut v0.1.0 release** — repo variables are configured. Run the release workflow.
4. **Write release post for claylo.dev** — blog post draft in `scratch/release-announcement-v0.1.0.md`, GitHub release note in `scratch/github-release-v0.1.0.md`.
5. **Generate postcard** — `claylo-graphics --config scratch/postcard-v0.1.0.yaml` (xl, codename "Bringing Up The Rear").
6. **Backport compact help to claylo-rs template** — the `disable_help_flag` + `HelpShort` pattern should go into the standard preset.

## Landmines

- **`commit.txt` is uncommitted.** The banner/help changes are staged but not committed. Run `gtxt` on a feature branch before anything else.
- **`explanatory-output-style` plugin has a broken hook.** The `unknown` version's `session-start.sh` was missing `+x`. Fixed locally with `chmod`, but it'll recur if the plugin cache is refreshed. Anthropic's bug, not ours.
- **`scratch/` and `dist/` are untracked.** Release prep artifacts (postcard yaml, release notes drafts, banner preview script, generated postcard images) live there. Don't accidentally commit them to main.
- **`assets/` has resized images.** `glossary.png` was resampled to 650px height to match `cc-index-main-only.png`. Both are untracked.
