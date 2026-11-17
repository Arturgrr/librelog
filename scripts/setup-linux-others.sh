#!/usr/bin/env bash

. "$(dirname "$0")/bash-guard.sh"

set -euo pipefail

# Fedora's names for the Debian packages below.
if [[ -f /etc/os-release ]] && (. /etc/os-release && [[ "${ID:-}" == fedora ]]); then
  sudo dnf install -y \
    clang \
    clang-devel \
    gtk3-devel \
    gtk4-devel \
    alsa-lib-devel \
    systemd-devel \
    pulseaudio-libs-devel \
    pipewire-devel \
    graphene-devel \
    pkgconf-pkg-config \
    patchelf \
    xdg-utils \
    cmake \
    curl \
    jq \
    libcurl-devel \
    unzip \
    vulkan-loader-devel \
    vulkan-headers \
    glslc \
    spirv-headers-devel
  exit 0
fi

sudo apt update
sudo apt-get install -y \
  clang \
  libclang-dev \
  libgtk-3-dev \
  libgtk-4-dev \
  libasound2-dev \
  libudev-dev \
  libpulse-dev \
  libpipewire-0.3-dev \
  libgraphene-1.0-dev \
  pkg-config \
  patchelf \
  xdg-utils \
  cmake \
  curl \
  jq \
  libcurl4-openssl-dev \
  unzip \
  libvulkan-dev \
  glslc \
  spirv-headers
