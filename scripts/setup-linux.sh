#!/usr/bin/env bash

. "$(dirname "$0")/bash-guard.sh"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE_MAJOR=22
RUST_VERSION=1.94.0
PNPM_VERSION=11.1.1

node_major() {
  local version=0
  if command -v node &> /dev/null; then
    version="$(node --version)"
    version="${version#v}"
    version="${version%%.*}"
  fi
  echo "$version"
}

setup_toolchains() {
  if [[ ! -f /etc/os-release ]]; then
    echo "Error: Linux setup supports Debian, Ubuntu and Fedora." >&2
    exit 1
  fi

  . /etc/os-release
  case "${ID:-}" in
    debian | ubuntu) PACKAGE_MANAGER=apt ;;
    fedora) PACKAGE_MANAGER=dnf ;;
    *)
      echo "Error: Linux setup supports Debian, Ubuntu and Fedora, not ${ID:-unknown}." >&2
      exit 1
      ;;
  esac

  case "$(uname -m)" in
    x86_64 | amd64 | aarch64 | arm64) ;;
    *)
      echo "Error: Linux setup supports x86_64 and ARM64 only." >&2
      exit 1
      ;;
  esac

  if [[ "$PACKAGE_MANAGER" == dnf ]]; then
    sudo dnf install -y ca-certificates curl libatomic
  else
    sudo apt-get update
    sudo apt-get install -y ca-certificates curl libatomic1
  fi

  if (($(node_major) < NODE_MAJOR)); then
    if [[ "$PACKAGE_MANAGER" == dnf ]]; then
      # Fedora 41 and later ship Node.js 22 or newer as their default.
      sudo dnf install -y nodejs npm
    else
      local nodesource_setup
      nodesource_setup="$(mktemp)"
      curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" -o "$nodesource_setup"
      sudo -E bash "$nodesource_setup"
      rm -f "$nodesource_setup"
      sudo apt-get install -y nodejs
    fi
  fi
  if (($(node_major) < NODE_MAJOR)); then
    echo "Error: Node.js $NODE_MAJOR or later is required; install it and run this script again." >&2
    exit 1
  fi

  local cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  if [[ -f "$cargo_home/env" ]]; then
    . "$cargo_home/env"
  else
    export PATH="$cargo_home/bin:$PATH"
  fi
  if ! command -v rustup &> /dev/null; then
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION"
  fi
  rustup toolchain install "$RUST_VERSION" --profile minimal

  export PNPM_HOME="${PNPM_HOME:-$HOME/.local/share/pnpm}"
  export PATH="$PNPM_HOME/bin:$PATH"
  if ! command -v pnpm &> /dev/null || [[ "$(pnpm --version)" != "$PNPM_VERSION" ]]; then
    curl -fsSL https://get.pnpm.io/install.sh \
      | env PNPM_VERSION="$PNPM_VERSION" SHELL="${SHELL:-/bin/bash}" sh -
  fi

  echo "Node $(node --version), Rust $(rustc --version), pnpm $(pnpm --version)"
}

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
  setup_toolchains
  bash "$SCRIPT_DIR/setup-linux-others.sh"
  bash "$SCRIPT_DIR/setup-linux-tauri.sh"
  bash "$SCRIPT_DIR/setup-devtools.sh"
elif [[ "$OSTYPE" == "darwin"* ]]; then
  echo "Error: macOS is not supported by this script"
  exit 1
else
  echo "Error: Unsupported OS: $OSTYPE"
  exit 1
fi
