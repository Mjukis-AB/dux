# DUX - Interactive Terminal Disk Usage Analyzer

An interactive, DaisyDisk-like terminal disk usage analyzer with rich 24-bit color UI, Unicode graphics, and drill-down navigation.

![DUX Screenshot](assets/screenshot.png)

## Features

- **Parallel scanning** using jwalk for fast filesystem traversal
- **24-bit RGB colors** with Catppuccin-inspired dark theme
- **Unicode graphics**: Box drawing, partial blocks for smooth size bars, folder icons
- **Interactive navigation**: Drill down into directories, expand/collapse, keyboard shortcuts
- **Real-time progress** with animated spinner during scanning

## Installation

### From crates.io

```bash
cargo install dux-cli
```

### From Homebrew (macOS/Linux)

```bash
brew tap mjukis-ab/tap
brew install dux
```

### From source

```bash
git clone https://github.com/mjukis-ab/dux
cd dux
cargo install --path dux-cli
```

## Development

```bash
# Build and run directly from the repo
cargo run -p dux-cli

# Run with arguments
cargo run -p dux-cli -- /path/to/directory

# Build release binary
cargo build --release -p dux-cli
# Binary at: target/release/dux
```

## Usage

```bash
# Analyze current directory
dux

# Analyze specific path
dux /path/to/directory

# Limit scan depth
dux -m 3 /path

# Follow symbolic links
dux -f /path

# Cross filesystem boundaries
dux -x /path
```

## Keyboard Navigation

| Key | Action |
|-----|--------|
| `↑`/`k` | Move up |
| `↓`/`j` | Move down |
| `→`/`l` | Expand directory |
| `←`/`h` | Collapse directory |
| `Space` | Toggle expand/collapse |
| `Tab`/`Shift-Tab` | Switch views |
| `Enter` | Drill down into directory |
| `Backspace`/`Esc` | Go back |
| `v` | Toggle multi-selection |
| `d` | Delete selected item(s) |
| `o` | Reveal selected item in Finder (macOS) |
| `?` | Show help |
| `q`/`Ctrl+C` | Quit |

## License

MIT
