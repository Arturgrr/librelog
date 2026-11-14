#!/usr/bin/env bash

. "$(dirname "$0")/bash-guard.sh"

set -euo pipefail

# Fedora's names for the Debian packages below.
if [[ -f /etc/os-release ]] && (. /etc/os-release && [[ "${ID:-}" == fedora ]]); then
  sudo dnf install -y \
    webkit2gtk4.1-devel \
    gcc \
    gcc-c++ \
    make \
    curl \
    wget \
    file \
    libxdo-devel \
    openssl-devel \
    libayatana-appindicator-gtk3-devel \
    librsvg2-devel \
    xdg-utils
  exit 0
fi

sudo apt-get -o Acquire::Retries=5 update
sudo apt-get -o Acquire::Retries=5 install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  xdg-utils
