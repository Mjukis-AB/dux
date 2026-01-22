# DUX - Interactive Terminal Disk Usage Analyzer

An interactive, DaisyDisk-like terminal disk usage analyzer with rich 24-bit color UI, Unicode graphics, and drill-down navigation.

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
| `Space`/`Tab` | Toggle expand/collapse |
| `Enter` | Drill down into directory |
| `Backspace`/`Esc` | Go back |
| `?` | Show help |
| `q`/`Ctrl+C` | Quit |

## License

MIT
