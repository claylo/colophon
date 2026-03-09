---
title: CLI Reference
description: Command-line interface reference for colophon
---

## Global options

| Option | Description |
|--------|-------------|
| `--verbose` | Enable verbose output |
| `--json` | Output in JSON format |
| `--version` | Print version information |
| `--help` | Print help |

## Commands

### `info`

Display version, build, and environment information.

```bash
colophon info
colophon info --json
```

### `completions`

Generate shell completions.

```bash
colophon completions bash
colophon completions zsh
colophon completions fish
colophon completions powershell
```


### `doctor`

Check configuration and environment health.

```bash
colophon doctor
```
