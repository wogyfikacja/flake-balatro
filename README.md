# Balatro Modding Environment

Nix flake for Balatro modding with wiki-based mod discovery.

## Setup

```bash
nix develop
balatro-mod-setup
balatro-update-wiki
```

## Commands

**Mod Management:**
- `balatro-install-mod <url>` - Install from Git/zip/local
- `balatro-list-mods` - List installed mods
- `balatro-remove-mod <name>` - Remove mod
- `balatro-launch` - Launch with mods (Linux)

**Wiki Integration:**
- `balatro-search-mods <query>` - Search mods
- `balatro-browse-mods [category]` - Browse by category
- `balatro-mod-info <name>` - Get mod details
- `balatro-install-from-wiki <name>` - Install from wiki

## Recent Fixes

- Fixed chrono import in Rust code
- Unicode-safe string truncation
- Removed unused dependencies
- Builds correctly with Nix

## Requirements

- Nix with flakes
- Steam with Balatro installed