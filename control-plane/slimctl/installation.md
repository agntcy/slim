# Installation

`slimctl` is available for multiple operating systems and architectures.

## Download from GitHub

1. Visit the [slimctl GitHub releases page](https://github.com/agntcy/slim/releases)
2. Download the asset matching your operating system and architecture (e.g., Linux, macOS, Windows)
3. Extract the downloaded archive and move the `slimctl` binary to a directory in your `PATH`

## Install with Homebrew (macOS)

If you are using macOS, you can install slimctl via Homebrew:

```bash
brew tap agntcy/slim
brew install slimctl
```

This will automatically download and install the latest version of slimctl for your system.

## Building from Source

```bash
# From repository root
cd control-plane
task control-plane:slimctl:build

# Binary will be at: .dist/bin/slimctl (from repository root)
```

### Prerequisites

- Go 1.20+
- Task (taskfile.dev)
