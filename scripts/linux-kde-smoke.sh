#!/usr/bin/env bash
# Starts a released Librelog RPM in a headless KDE Plasma 6 Wayland session
# and checks what only a KDE session exercises: kde_smoke.yaml runs it in a
# Fedora container every night and on demand, never per pull request (the
# packages it installs are large).
#
#   linux-kde-smoke.sh          installs the session's packages and the RPM,
#                               then runs `session` under dbus-run-session
#   linux-kde-smoke.sh session  starts PipeWire and KWin (virtual output,
#                               XWayland), which runs `checks` as its client
#   linux-kde-smoke.sh checks   starts Librelog and checks it
#
# The checks (tier 1, nothing pressed or approved):
#   - Librelog is still running after $STAY_SECONDS;
#   - it registered its app id with the portal's host app registry;
#   - its tray item is registered with Plasma's StatusNotifierWatcher;
#   - KDE bound its dictation shortcut through the Global Shortcuts portal.
#
# RPM is a local package to test. Without it, the newest release with an
# x86_64 RPM is downloaded from GitHub Releases (RELEASE_TAG picks another)
# and checked against its SHA256SUMS.txt. Needs root in a Fedora container;
# the session runs as the user `smoke`.

. "$(dirname "$0")/bash-guard.sh"

set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
REPOSITORY="${GITHUB_REPOSITORY:-Arturgrr/librelog}"
APP_ID=com.arturgrr.librelog
STAY_SECONDS="${STAY_SECONDS:-60}"
WORK="${WORK:-/tmp/kde-smoke}"
LOG="$WORK/librelog.log"
RESULT="$WORK/result"

fail() {
  echo "::error::$*"
  exit 1
}

# Strips the colours tracing writes to stderr.
plain() {
  sed -e 's/\x1b\[[0-9;]*m//g' "$@"
}

download_rpm() {
  local releases url name sums
  mkdir -p "$WORK/rpm"
  # Unauthenticated: one request, well inside GitHub's anonymous limit.
  releases="$(curl -fsSL "https://api.github.com/repos/$REPOSITORY/releases?per_page=20")"
  # Pre-releases count: Linux builds are betas.
  read -r url name < <(jq -r --arg tag "${RELEASE_TAG:-}" '
    [.[] | select(.draft | not) | select($tag == "" or .tag_name == $tag)
      | .assets[] | select(.name | test("\\.x86_64\\.rpm$"))][0]
    | "\(.browser_download_url) \(.name)"' <<<"$releases")
  [[ -n "$url" && "$url" != null ]] || fail "no release ${RELEASE_TAG:-} has an x86_64 RPM"
  echo "Testing $name"
  curl -fsSL --output "$WORK/rpm/$name" "$url"
  sums="$(dirname "$url")/SHA256SUMS.txt"
  if curl -fsSL --output "$WORK/rpm/SHA256SUMS.txt" "$sums"; then
    (cd "$WORK/rpm" && sha256sum -c SHA256SUMS.txt --ignore-missing)
  else
    echo "::warning::$sums is missing; the RPM is not checked against it"
  fi
  RPM="$WORK/rpm/$name"
}

prepare() {
  rm -rf "$WORK"
  mkdir -p "$WORK"
  # kwin-wayland: the compositor, with kglobalaccel built in (Plasma 6).
  # plasma-workspace: the kded module that is Plasma's StatusNotifierWatcher.
  # xdg-desktop-portal-kde: GlobalShortcuts, RemoteDesktop and the rest.
  # python3-gobject-base: the stand-in tray host below. glib2: gdbus.
  dnf install -y --setopt=install_weak_deps=False \
    kwin-wayland xorg-x11-server-Xwayland plasma-workspace kf6-kded \
    xdg-desktop-portal xdg-desktop-portal-kde \
    pipewire wireplumber mesa-dri-drivers dbus-daemon dbus-tools \
    glib2 python3-gobject-base util-linux shadow-utils curl jq
  if [[ -z "${RPM:-}" ]]; then
    download_rpm
  fi
  dnf install -y "$RPM"
  rpm -q --qf '%{NAME} %{VERSION}-%{RELEASE}\n' librelog
  rpm -q --qf '%{NAME} %{VERSION}-%{RELEASE}\n' kwin-wayland xdg-desktop-portal xdg-desktop-portal-kde

  # The session runs as a regular user, as a desktop does, from a copy of
  # this script it can read.
  id smoke >/dev/null 2>&1 || useradd --create-home smoke
  install -m 0755 "$SCRIPT" "$(dirname "$SCRIPT")/bash-guard.sh" "$WORK/"
  chown -R smoke "$WORK"
  set +e
  runuser -u smoke -- env WORK="$WORK" STAY_SECONDS="$STAY_SECONDS" \
    dbus-run-session -- bash "$WORK/$(basename "$SCRIPT")" session
  set -e
  [[ -f "$RESULT" ]] || fail "the KDE session ended before the checks ran"
  [[ "$(cat "$RESULT")" == passed ]] || fail "KDE smoke checks failed: $(cat "$RESULT")"
  echo "KDE smoke checks passed"
}

session() {
  export XDG_RUNTIME_DIR="$WORK/runtime" XDG_CONFIG_HOME="$WORK/config"
  export XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland XDG_SESSION_DESKTOP=KDE
  export KDE_FULL_SESSION=true KDE_SESSION_VERSION=6
  mkdir -p -m 0700 "$XDG_RUNTIME_DIR"
  mkdir -p "$XDG_CONFIG_HOME"

  # KDE keeps a shortcut the user already has over the app's suggestion and
  # may otherwise ask about a new one. Seeding Librelog's default dictation
  # shortcut (bindings.rs names it `dictate-<trigger>`) lets BindShortcuts
  # answer without a dialog nobody can click here.
  cat >"$XDG_CONFIG_HOME/kglobalshortcutsrc" <<EOF
[$APP_ID]
_k_friendly_name=Librelog
dictate-CTRL-ALT-space=Ctrl+Alt+Space,Ctrl+Alt+Space,Dictate with Librelog
EOF

  pipewire &
  wireplumber &
  # KWin starts `checks` with WAYLAND_DISPLAY and DISPLAY set, and exits when
  # it does. The result goes through $RESULT: KWin's exit status is its own.
  kwin_wayland --virtual --xwayland --no-lockscreen --width 1280 --height 800 \
    --exit-with-session "bash $SCRIPT checks" >"$WORK/kwin.log" 2>&1 || true
  echo "::group::KWin log"
  cat "$WORK/kwin.log"
  echo "::endgroup::"
}

name_has_owner() {
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner "$1" 2>/dev/null | grep -q true
}

wait_for_name() {
  local name="$1"
  for _ in $(seq 1 40); do
    name_has_owner "$name" && return 0
    sleep 0.5
  done
  return 1
}

# Plasma's panel is the tray host; this stands in for it so that the
# watcher reports a host and libappindicator registers its item.
tray_host() {
  python3 -I -c '
import os
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
name = "org.kde.StatusNotifierHost-%d" % os.getpid()
bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
    "RequestName", GLib.Variant("(su)", (name, 0)), None, Gio.DBusCallFlags.NONE, 5000, None)
bus.call_sync("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher",
    "org.kde.StatusNotifierWatcher", "RegisterStatusNotifierHost",
    GLib.Variant("(s)", (name,)), None, Gio.DBusCallFlags.NONE, 5000, None)
GLib.MainLoop().run()
' &
}

checks() {
  local failures=() pid status
  # Bus-activated portals start with the bus's environment, which predates
  # KWin: give them its display and desktop.
  dbus-update-activation-environment --all
  echo "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-} DISPLAY=${DISPLAY:-}"

  kded6 >"$WORK/kded.log" 2>&1 &
  if ! wait_for_name org.kde.StatusNotifierWatcher; then
    gdbus call --session --dest org.kde.kded6 --object-path /kded \
      --method org.kde.kded6.loadModule statusnotifierwatcher || true
    wait_for_name org.kde.StatusNotifierWatcher || failures+=("no StatusNotifierWatcher")
  fi
  tray_host

  ONBOARDING=0 RUST_BACKTRACE=1 /usr/bin/librelog >"$LOG" 2>&1 &
  pid=$!
  for _ in $(seq 1 "$STAY_SECONDS"); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
  done

  if ! kill -0 "$pid" 2>/dev/null; then
    wait "$pid" && status=0 || status=$?
    failures+=("Librelog exited with status $status within ${STAY_SECONDS}s")
  fi

  plain "$LOG" | grep -q "registered with the desktop portal" ||
    failures+=("Librelog did not register $APP_ID with the portal")

  local items
  items="$(gdbus call --session --dest org.kde.StatusNotifierWatcher \
    --object-path /StatusNotifierWatcher --method org.freedesktop.DBus.Properties.Get \
    org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems 2>&1 || true)"
  echo "Tray items: $items"
  # Nothing else in this session has a tray item.
  grep -q "'[^']" <<<"$items" || failures+=("no tray item registered")

  plain "$LOG" | grep "the desktop bound a Librelog shortcut" | grep -q "id=\"\?dictate-" ||
    failures+=("KDE did not bind the dictation shortcut")
  echo "kglobalaccel components:"
  gdbus call --session --dest org.kde.kglobalaccel --object-path /kglobalaccel \
    --method org.kde.KGlobalAccel.allComponents || true

  kill "$pid" 2>/dev/null || true
  echo "::group::Librelog log"
  plain "$LOG"
  echo "::endgroup::"

  if ((${#failures[@]})); then
    printf '%s; ' "${failures[@]}" >"$RESULT"
    printf '::error::%s\n' "${failures[@]}"
  else
    echo passed >"$RESULT"
  fi
}

case "${1:-}" in
  "") prepare ;;
  session) session ;;
  checks) checks ;;
  *)
    echo "Usage: $0 [session | checks]" >&2
    exit 2
    ;;
esac
