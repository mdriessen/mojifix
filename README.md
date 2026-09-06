# mojifix

Safely repair common UTF-8 / Windows-1252 mojibake (double-encoded text).

`mojifix` detects and reverses text that was UTF-8 → decoded as Windows-1252 → re-encoded as UTF-8. Handles single, double and triple encoding iteratively.

## What it fixes

- `HÃƒÂ©llo` → `Héllo`, `CafÃƒÂ©` → `Café`, `MÃƒÂ¼nchen` → `München`
- Smart quotes: `Ã¢â‚¬Å“HelloÃ¢â‚¬Â` → `“Hello”`, `Ã¢â‚¬â„¢` → `’`
- Dashes/ellipsis: `Ã¢â‚¬â€œ` → `–`, `Ã¢â‚¬â€` → `—`, `Ã¢â‚¬Â¦` → `…`
- Symbols/emoji: `Ã‚Â©` → `©`, `Ã°Å¸ËœÅ ` → `😊`, `Ã°Å¸Å’Â` → `🌍`
- C1 controls (`U+0080`-`U+009F`) and NBSP are handled; truly non-Windows-1252 characters are left as-is.

Conservative mode (default) only repairs when mojibake score decreases.

## Prerequisites

- Rust stable toolchain and Cargo via [rustup](https://rustup.rs)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  # verify
  rustc --version
  cargo --version
  ```
  Requires Rust ≥ 1.70 (edition 2021).

## Install

```bash
# binary installed in ~/.cargo/bin/mojifix
cargo install --path .
```

## Usage

```
mojifix [OPTIONS] <INPUT>

Options:
  -o, --output <OUTPUT>  Output file
      --in-place         Replace input file atomically
      --dry-run          Show what would change without writing
      --conservative     Only repair when highly confident (default: true)
  -h, --help             Print help
  -V, --version          Print version
```

Examples:

```bash
# Dry run
mojifix input.txt --dry-run

# Write to new file
mojifix input.txt --output fixed.txt

# In-place atomic replace
mojifix input.txt --in-place

# Disable conservative check
mojifix input.txt --output fixed.txt --conservative=false
```
