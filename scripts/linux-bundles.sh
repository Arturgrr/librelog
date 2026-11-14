#!/usr/bin/env bash
# Builds, checks and smoke-tests the stable app's Linux packages (RPM,
# AppImage, deb) for x86_64. release.yaml and desktop_ci.yaml's linux_bundle
# job run the same steps:
#
#   linux-bundles.sh build    cargo xtask prepare-binaries, tauri build, then
#                             the AppImage repack and the package versions
#   linux-bundles.sh verify   package metadata, requirements and contents
#   linux-bundles.sh smoke    installs each package in a clean container
#                             (Fedora for the RPM and AppImage, Ubuntu for the
#                             deb) and launches the app headless
#   linux-bundles.sh assets   prints the files a release uploads
#
# Without TAURI_SIGNING_PRIVATE_KEY the build has no updater artifacts and the
# AppImage stays unsigned; with REQUIRE_SIGNATURE=1, verify fails unless the
# AppImage's updater signature matches the public key in tauri.conf.json.

. "$(dirname "$0")/bash-guard.sh"

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=x86_64-unknown-linux-gnu
SRC_TAURI="$ROOT/apps/desktop/src-tauri"
# apps/desktop/src-tauri/.cargo/config.toml puts the build there unless
# CARGO_TARGET_DIR moves it (the workflows use the workspace's, which the Rust
# cache saves).
BUNDLE_DIR="${CARGO_TARGET_DIR:-$SRC_TAURI/target}/$TARGET/release/bundle"
VERSION="$(jq -r .version "$ROOT/apps/desktop/package.json")"
PACKAGE_VERSION="$(node "$ROOT/scripts/release.mjs" linux-package-version "$VERSION")"
# The containers read these through `docker run --env NAME`.
export PACKAGE_VERSION
FEDORA_IMAGE=fedora:latest
UBUNTU_IMAGE=ubuntu:24.04
# Installed by every package; the smoke tests run each.
BINARIES=(librelog librelog-cli librelog-chrome-native-host)
DESKTOP_FILE=/usr/share/applications/Librelog.desktop
ICON=/usr/share/icons/hicolor/512x512/apps/librelog.png

single() {
  local pattern="$1"
  local matches
  shopt -s nullglob
  # shellcheck disable=SC2206 # the pattern is a glob on purpose
  matches=($pattern)
  shopt -u nullglob
  if [[ ${#matches[@]} -ne 1 ]]; then
    echo "::error::Expected one $pattern, found ${#matches[@]}" >&2
    exit 1
  fi
  echo "${matches[0]}"
}

fail() {
  echo "::error::$*" >&2
  exit 1
}

build() {
  export APP_VERSION="${APP_VERSION:-$VERSION}"
  TAURI_ENV_TARGET_TRIPLE="$TARGET" cargo xtask prepare-binaries

  local sidecars=(binaries/librelog-chrome-native-host resources/cli/librelog-cli)
  # crates/xtask builds the built-in model server only for the targets it
  # ships on; once it builds this one, the packages carry it too.
  if [[ -f "$SRC_TAURI/binaries/librelog-llama-server-$TARGET" ]]; then
    sidecars+=(binaries/librelog-llama-server)
  fi
  local config="$SRC_TAURI/tauri.conf.stable.json"
  local updater_artifacts=true
  [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]] || updater_artifacts=false
  jq --argjson updater "$updater_artifacts" '
    .bundle.externalBin = ((.bundle.externalBin // []) + $ARGS.positional | unique)
    | if $updater then . else .bundle.createUpdaterArtifacts = false end
  ' --args "${sidecars[@]}" <"$config" >"$config.tmp"
  mv "$config.tmp" "$config"

  (
    cd "$ROOT/apps/desktop"
    pnpm tauri build --ci --target "$TARGET" --bundles rpm,appimage,deb \
      --config src-tauri/tauri.conf.stable.json
  )

  ARCH=x86_64 node "$ROOT/scripts/repack-linux-appimage.mjs" "$BUNDLE_DIR/appimage"
  set_package_versions
}

# Tauri writes the app's semver into both packages; see linuxPackageVersion
# in scripts/release.mjs for why they need a tilde. The packages' own updater
# signatures go too: they would not match, and only the AppImage updates
# itself.
set_package_versions() {
  local deb rpm
  deb="$(single "$BUNDLE_DIR/deb/*.deb")"
  rpm="$(single "$BUNDLE_DIR/rpm/*.rpm")"
  rm -f "$deb.sig" "$rpm.sig"
  if [[ "$PACKAGE_VERSION" == "$VERSION" ]]; then
    return
  fi

  local work
  work="$(mktemp -d)"
  dpkg-deb -R "$deb" "$work/deb"
  sed -i "s/^Version: .*/Version: $PACKAGE_VERSION/" "$work/deb/DEBIAN/control"
  dpkg-deb --root-owner-group -b "$work/deb" "$deb"
  rm -rf "$work"

  docker run --rm -i \
    --volume "$(dirname "$rpm"):/rpm" \
    --env RPM_NAME="$(basename "$rpm")" \
    --env PACKAGE_VERSION \
    --env HOST_OWNER="$(id -u):$(id -g)" \
    "$FEDORA_IMAGE" bash -euxo pipefail -s <<'EOF'
dnf install -y --setopt=install_weak_deps=False rpm-build rpmrebuild
rpmrebuild --package --notest-install --directory=/tmp/rebuilt \
  --change-spec-preamble="sed -e 's/^Version:.*/Version: ${PACKAGE_VERSION}/'" \
  "/rpm/$RPM_NAME"
rebuilt=(/tmp/rebuilt/*/*.rpm)
[[ ${#rebuilt[@]} -eq 1 ]]
install -m 0644 "${rebuilt[0]}" "/rpm/$RPM_NAME"
chown "$HOST_OWNER" "/rpm/$RPM_NAME"
EOF
}

expect_lines() {
  local what="$1" actual="$2"
  shift 2
  local expected
  for expected in "$@"; do
    grep -Fxq -- "$expected" <<<"$actual" || fail "$what lacks $expected"
  done
}

verify() {
  local appimage deb rpm
  appimage="$(single "$BUNDLE_DIR/appimage/*.AppImage")"
  deb="$(single "$BUNDLE_DIR/deb/*.deb")"
  rpm="$(single "$BUNDLE_DIR/rpm/*.rpm")"

  [[ "$(basename "$appimage")" == *"_${VERSION}_"* ]] ||
    fail "$(basename "$appimage") does not carry version $VERSION"
  file "$appimage" | grep -Fq x86-64 || fail "$appimage is not an x86-64 executable"
  if [[ "${REQUIRE_SIGNATURE:-}" == 1 ]]; then
    node "$ROOT/scripts/release.mjs" verify-updater-signature \
      "$appimage" "$appimage.sig" "$SRC_TAURI/tauri.conf.json"
  fi

  [[ "$(dpkg-deb --field "$deb" Architecture)" == amd64 ]] || fail "$deb is not amd64"
  [[ "$(dpkg-deb --field "$deb" Version)" == "$PACKAGE_VERSION" ]] ||
    fail "$deb is not at version $PACKAGE_VERSION"
  local depends
  depends="$(dpkg-deb --field "$deb" Depends | tr ',' '\n' | sed 's/^ *//; s/ *$//')"
  mapfile -t expected < <(jq -r '.bundle.linux.deb.depends[]' "$SRC_TAURI/tauri.conf.json")
  expect_lines "$deb's Depends" "$depends" "${expected[@]}"
  local files
  files="$(dpkg-deb --fsys-tarfile "$deb" | tar -t | sed 's|^\./|/|')"
  expect_lines "$deb" "$files" "${BINARIES[@]/#//usr/bin/}" "$DESKTOP_FILE" "$ICON"

  mapfile -t expected < <(jq -r '.bundle.linux.rpm.depends[]' "$SRC_TAURI/tauri.conf.json")
  docker run --rm -i \
    --volume "$(dirname "$rpm"):/rpm:ro" \
    --env RPM_NAME="$(basename "$rpm")" \
    --env PACKAGE_VERSION \
    --env EXPECTED_REQUIRES="$(printf '%s\n' "${expected[@]}")" \
    --env EXPECTED_FILES="$(printf '%s\n' "${BINARIES[@]/#//usr/bin/}" "$DESKTOP_FILE" "$ICON")" \
    "$FEDORA_IMAGE" bash -euo pipefail -s <<'EOF'
rpm="/rpm/$RPM_NAME"
evr="$(rpm -qp --qf '%{VERSION}-%{RELEASE}.%{ARCH}' "$rpm")"
echo "$RPM_NAME: $evr"
[[ "$evr" == "$PACKAGE_VERSION-1.x86_64" ]] || { echo "::error::$RPM_NAME is $evr, not $PACKAGE_VERSION-1.x86_64"; exit 1; }
requires="$(rpm -qpR "$rpm")"
files="$(rpm -qpl "$rpm")"
echo "$requires"
status=0
while read -r name; do
  grep -Fxq -- "$name" <<<"$requires" || { echo "::error::$RPM_NAME does not require $name"; status=1; }
done <<<"$EXPECTED_REQUIRES"
while read -r path; do
  grep -Fxq -- "$path" <<<"$files" || { echo "::error::$RPM_NAME lacks $path"; status=1; }
done <<<"$EXPECTED_FILES"
exit "$status"
EOF
}

# Run inside the containers: an app still running at the timeout passes, as
# one that crashes or exits on startup does not.
read -r -d '' LAUNCH <<'EOF' || true
check_libraries() {
  local missing
  missing="$(LD_LIBRARY_PATH="${2:-}" ldd "$1" | grep "not found" || true)"
  [[ -z "$missing" ]] || { echo "::error::$1 links libraries the system lacks:"; echo "$missing"; exit 1; }
}
launch() {
  set +e
  timeout --signal=TERM 30s xvfb-run -a dbus-run-session -- "$@" >/tmp/librelog.log 2>&1
  local status=$?
  set -e
  cat /tmp/librelog.log
  [[ $status -eq 124 ]] || { echo "::error::Librelog exited before the smoke timeout with status $status"; exit 1; }
}
EOF

smoke() {
  local appimage deb rpm
  appimage="$(single "$BUNDLE_DIR/appimage/*.AppImage")"
  deb="$(single "$BUNDLE_DIR/deb/*.deb")"
  rpm="$(single "$BUNDLE_DIR/rpm/*.rpm")"
  local common=(
    --rm -i
    --volume "$BUNDLE_DIR:/bundle:ro"
    --env LAUNCH
    --env BINARIES="${BINARIES[*]}"
    --env DESKTOP_FILE
    --env RUST_BACKTRACE=1
  )
  export LAUNCH DESKTOP_FILE

  echo "::group::RPM on $FEDORA_IMAGE"
  # dnf resolves every Requires against Fedora's own repositories here, so a
  # misspelled or missing package name fails the install.
  docker run "${common[@]}" --env PACKAGE="/bundle/rpm/$(basename "$rpm")" \
    "$FEDORA_IMAGE" bash -euxo pipefail -s <<'EOF'
eval "$LAUNCH"
dnf install -y "$PACKAGE"
dnf install -y xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon mesa-dri-drivers util-linux which
desktop-file-validate "$DESKTOP_FILE"
for binary in $BINARIES; do check_libraries "/usr/bin/$binary"; done
librelog-cli --help
launch /usr/bin/librelog
EOF
  echo "::endgroup::"

  echo "::group::deb on $UBUNTU_IMAGE"
  docker run "${common[@]}" --env PACKAGE="/bundle/deb/$(basename "$deb")" \
    "$UBUNTU_IMAGE" bash -euxo pipefail -s <<'EOF'
eval "$LAUNCH"
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y "$PACKAGE"
apt-get install -y dbus-x11 xvfb
for binary in $BINARIES; do check_libraries "/usr/bin/$binary"; done
librelog-cli --help
launch /usr/bin/librelog
EOF
  echo "::endgroup::"

  echo "::group::AppImage on $FEDORA_IMAGE"
  # What a Fedora desktop has and AppImages leave to the system (AppImage's
  # excludelist: GL, X11, fontconfig, ALSA, PipeWire; the repack adds Wayland
  # and PipeWire's SPA modules).
  docker run "${common[@]}" --env PACKAGE="/bundle/appimage/$(basename "$appimage")" \
    "$FEDORA_IMAGE" bash -euxo pipefail -s <<'EOF'
eval "$LAUNCH"
dnf install -y gtk3 pipewire-libs alsa-lib mesa-libGL mesa-libEGL mesa-libgbm \
  xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon mesa-dri-drivers util-linux which
cd /tmp
cp "$PACKAGE" Librelog.AppImage
chmod +x Librelog.AppImage
./Librelog.AppImage --appimage-extract >/dev/null
check_libraries squashfs-root/usr/bin/librelog squashfs-root/usr/lib
launch ./Librelog.AppImage --appimage-extract-and-run
EOF
  echo "::endgroup::"
}

assets() {
  single "$BUNDLE_DIR/appimage/*.AppImage"
  single "$BUNDLE_DIR/appimage/*.AppImage.sig"
  single "$BUNDLE_DIR/deb/*.deb"
  single "$BUNDLE_DIR/rpm/*.rpm"
}

case "${1:-}" in
  build) build ;;
  verify) verify ;;
  smoke) smoke ;;
  assets) assets ;;
  *)
    echo "Usage: $0 build | verify | smoke | assets" >&2
    exit 1
    ;;
esac
