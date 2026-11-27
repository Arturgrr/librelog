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
STABLE_CONFIG="$SRC_TAURI/tauri.conf.stable.json"
IDENTIFIER="$(jq -r .identifier "$STABLE_CONFIG")"
# The launcher, named by the app id the desktop portals know Librelog by;
# GNOME before 51 refuses one that is not reverse-DNS. The bundler's own
# entry, named after the product, stays hidden.
DESKTOP_FILE="/usr/share/applications/$IDENTIFIER.desktop"
HIDDEN_DESKTOP_FILE="/usr/share/applications/$(jq -r .productName "$STABLE_CONFIG").desktop"
export DESKTOP_FILE HIDDEN_DESKTOP_FILE IDENTIFIER
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
  # The app carries its own OpenSSL: linked against this machine's
  # libssl.so.3, it does not start on a distribution that has moved on
  # (Fedora 45 ships OpenSSL 4 only).
  local openssl_libs
  openssl_libs="/usr/lib/$(gcc -dumpmachine)"
  [[ -f "$openssl_libs/libssl.a" && -f "$openssl_libs/libcrypto.a" ]] ||
    fail "$openssl_libs has no static OpenSSL to link into the app (install libssl-dev)"
  export OPENSSL_STATIC=1 OPENSSL_LIB_DIR="$openssl_libs" OPENSSL_INCLUDE_DIR=/usr/include/openssl
  TAURI_ENV_TARGET_TRIPLE="$TARGET" cargo xtask prepare-binaries

  local sidecars=(binaries/librelog-chrome-native-host resources/cli/librelog-cli)
  # crates/xtask builds the built-in model server only for the targets it
  # ships on. A package for one of those without it would ship with built-in
  # models silently missing.
  if [[ -f "$SRC_TAURI/binaries/librelog-llama-server-$TARGET" ]]; then
    sidecars+=(binaries/librelog-llama-server)
  elif [[ "$TARGET" == x86_64-unknown-linux-gnu ]]; then
    fail "cargo xtask prepare-binaries did not build binaries/librelog-llama-server-$TARGET"
  fi
  local config="$STABLE_CONFIG"
  local updater_artifacts=true
  [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]] || updater_artifacts=false
  local entry appimage_template
  entry="$(launcher_entry)"
  appimage_template="$(appimage_entry)"
  # Tauri merges this over tauri.conf.json object by object, so the packages
  # keep their settings there and gain the launcher. The bundler writes the
  # RPM uncompressed: its own compression takes most of an hour on this app,
  # and set_package_versions rebuilds the RPM with rpmbuild, which compresses
  # it in seconds.
  jq --argjson updater "$updater_artifacts" \
    --arg desktop "$DESKTOP_FILE" --arg entry "$entry" '
    .bundle.externalBin = ((.bundle.externalBin // []) + $ARGS.positional | unique)
    | if $updater then . else .bundle.createUpdaterArtifacts = false end
    | .bundle.linux.deb.files[$desktop] = $entry
    | .bundle.linux.rpm.files[$desktop] = $entry
    | .bundle.linux.rpm.compression = {type: "none"}
  ' --args "${sidecars[@]}" <"$config" >"$config.tmp"
  mv "$config.tmp" "$config"

  (
    cd "$ROOT/apps/desktop"
    pnpm tauri build --ci --target "$TARGET" --bundles rpm,deb \
      --config src-tauri/tauri.conf.stable.json
    # The AppImage is bundled apart because the bundler gives it the deb's
    # desktop entry, and the deb's is the hidden one. An AppImage has no
    # other entry inside it, and appimagetool refuses one without Categories.
    pnpm tauri bundle --ci --target "$TARGET" --bundles appimage \
      --config src-tauri/tauri.conf.stable.json \
      --config "$(jq -cn --arg template "$appimage_template" '{bundle: {linux: {deb: {desktopTemplate: $template}}}}')"
  )

  ARCH=x86_64 node "$ROOT/scripts/repack-linux-appimage.mjs" "$BUNDLE_DIR/appimage"
  set_package_versions
}

# Fills in linux/app.desktop.in from the stable configuration and prints
# the file's path.
launcher_entry() {
  local exec scheme mime out
  exec="$(jq -r .mainBinaryName "$STABLE_CONFIG")"
  mime=""
  for scheme in $(jq -r '.plugins["deep-link"].desktop.schemes[]?' "$STABLE_CONFIG"); do
    mime+="x-scheme-handler/$scheme;"
  done
  out="${CARGO_TARGET_DIR:-$SRC_TAURI/target}/linux-entry/$IDENTIFIER.desktop"
  mkdir -p "$(dirname "$out")"
  sed -e "s|@NAME@|$(jq -r .productName "$STABLE_CONFIG")|" \
    -e "s|@COMMENT@|$(jq -r .bundle.shortDescription "$SRC_TAURI/tauri.conf.json")|" \
    -e "s|@EXEC@|$exec|g" \
    -e "s|@ICON@|$exec|" \
    -e "s|@MIME_TYPE@|$mime|" \
    -e "s|@IDENTIFIER@|$IDENTIFIER|" \
    "$SRC_TAURI/linux/app.desktop.in" >"$out"
  if [[ -z "$mime" ]]; then
    sed -i '/^MimeType=$/d' "$out"
  fi
  echo "$out"
}

# The AppImage's entry, linux/appimage.desktop with the app id filled in
# (the bundler's template knows no identifier); prints the file's path.
appimage_entry() {
  local out="${CARGO_TARGET_DIR:-$SRC_TAURI/target}/linux-entry/appimage.desktop"
  mkdir -p "$(dirname "$out")"
  sed -e "s|@IDENTIFIER@|$IDENTIFIER|" "$SRC_TAURI/linux/appimage.desktop" >"$out"
  echo "$out"
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

  if [[ "$PACKAGE_VERSION" != "$VERSION" ]]; then
    local work
    work="$(mktemp -d)"
    dpkg-deb -R "$deb" "$work/deb"
    sed -i "s/^Version: .*/Version: $PACKAGE_VERSION/" "$work/deb/DEBIAN/control"
    dpkg-deb --root-owner-group -b "$work/deb" "$deb"
    rm -rf "$work"
  fi

  # Every RPM is rebuilt, a stable version's too: the bundler left its
  # payload uncompressed (see build).
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
  expect_lines "$deb" "$files" "${BINARIES[@]/#//usr/bin/}" "$DESKTOP_FILE" "$HIDDEN_DESKTOP_FILE" "$ICON"
  local entries
  entries="$(mktemp -d)"
  (cd "$entries" && "$appimage" --appimage-extract '*.desktop' >/dev/null)
  local appimage_entries=("$entries"/squashfs-root/*.desktop)
  [[ ${#appimage_entries[@]} -eq 1 && -f "${appimage_entries[0]}" ]] ||
    fail "$appimage does not carry one desktop entry"
  grep -Fxq "StartupWMClass=$IDENTIFIER" "${appimage_entries[0]}" ||
    fail "$appimage's desktop entry does not claim the window class $IDENTIFIER"
  rm -rf "$entries/squashfs-root"
  mkdir -p "$entries/deb"
  dpkg-deb --fsys-tarfile "$deb" | tar -x -C "$entries/deb" ".$DESKTOP_FILE" ".$HIDDEN_DESKTOP_FILE"

  mapfile -t expected < <(jq -r '.bundle.linux.rpm.depends[]' "$SRC_TAURI/tauri.conf.json")
  docker run --rm -i \
    --volume "$(dirname "$rpm"):/rpm:ro" \
    --volume "$entries:/entries:ro" \
    --env RPM_NAME="$(basename "$rpm")" \
    --env PACKAGE_VERSION \
    --env DESKTOP_FILE --env HIDDEN_DESKTOP_FILE --env IDENTIFIER \
    --env EXPECTED_REQUIRES="$(printf '%s\n' "${expected[@]}")" \
    --env EXPECTED_FILES="$(printf '%s\n' "${BINARIES[@]/#//usr/bin/}" "$DESKTOP_FILE" "$HIDDEN_DESKTOP_FILE" "$ICON")" \
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
[[ "$status" -eq 0 ]] || exit "$status"
dnf install -y --setopt=install_weak_deps=False desktop-file-utils cpio >/dev/null
# The RPM's entries stay inside the container: files it wrote into the
# mounted folder would be root's, and the runner could not remove them.
mkdir -p /tmp/entries/rpm
ln -s /entries/deb /tmp/entries/deb
(cd /tmp/entries/rpm && rpm2cpio "$rpm" | cpio -id --quiet ".$DESKTOP_FILE" ".$HIDDEN_DESKTOP_FILE")
# Exactly one launcher per package: the reverse-DNS entry, valid and shown;
# the bundler's, named after the product, hidden and claiming no window.
for package in deb rpm; do
  launcher="/tmp/entries/$package$DESKTOP_FILE"
  hidden="/tmp/entries/$package$HIDDEN_DESKTOP_FILE"
  desktop-file-validate "$launcher" || { echo "::error::$package: $DESKTOP_FILE does not validate"; status=1; }
  desktop-file-validate "$hidden" || { echo "::error::$package: $HIDDEN_DESKTOP_FILE does not validate"; status=1; }
  if grep -q '^NoDisplay=true' "$launcher"; then echo "::error::$package: $DESKTOP_FILE is hidden"; status=1; fi
  grep -q '^NoDisplay=true' "$hidden" || { echo "::error::$package: $HIDDEN_DESKTOP_FILE is shown"; status=1; }
  if grep -q '^StartupWMClass=' "$hidden"; then echo "::error::$package: $HIDDEN_DESKTOP_FILE claims the window"; status=1; fi
  # GTK names the windows after the app id (set_prgname in the app's lib.rs).
  grep -Fxq "StartupWMClass=$IDENTIFIER" "$launcher" || { echo "::error::$package: $DESKTOP_FILE does not claim the window class $IDENTIFIER"; status=1; }
done
exit "$status"
EOF
  rm -rf "$entries"
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
  export LAUNCH

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
desktop-file-validate "$DESKTOP_FILE"
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
  libwayland-client libwayland-cursor libwayland-egl libwayland-server \
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
