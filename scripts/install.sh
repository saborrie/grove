#!/bin/sh
# Install grove from its GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/saborrie/grove/main/scripts/install.sh | sh
#
# Environment:
#   GROVE_VERSION       version to install (default: the latest release)
#   GROVE_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#
# POSIX sh on purpose: this is the one file that has to run before anything
# else is known to be present.

set -eu

REPO="saborrie/grove"
BIN="grove"
VERSION="${GROVE_VERSION:-latest}"
INSTALL_DIR="${GROVE_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"; }

need curl
need tar

# --- 1. platform -------------------------------------------------------------

os="$(uname -s)"
[ "$os" = "Linux" ] || die "the released binaries are Linux only (this is $os).
Build from source instead: https://github.com/${REPO}#from-source"

case "$(uname -m)" in
    x86_64 | amd64) target="x86_64-unknown-linux-musl" ;;
    aarch64 | arm64) target="aarch64-unknown-linux-musl" ;;
    *) die "unsupported architecture: $(uname -m)" ;;
esac

# --- 2. version --------------------------------------------------------------

if [ "$VERSION" = "latest" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep -m1 '"tag_name":' | cut -d'"' -f4)"
    [ -n "$VERSION" ] || die "could not work out the latest version — set GROVE_VERSION"
fi

asset="${BIN}-${VERSION}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"

# --- 3. download and verify --------------------------------------------------

tmp="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '$tmp'" EXIT INT TERM

say "==> downloading grove ${VERSION} (${target})"
curl -fsSL --retry 3 -o "${tmp}/${asset}" "$url" \
    || die "no such release: $url"
curl -fsSL --retry 3 -o "${tmp}/${asset}.sha256" "${url}.sha256" \
    || die "release is missing its checksum — refusing to install"

if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "${asset}.sha256" >/dev/null) \
        || die "checksum mismatch — the download is not what the release says it is"
    say "==> checksum ok"
else
    say "==> sha256sum not available, skipping verification"
fi

tar -xzf "${tmp}/${asset}" -C "$tmp"

# --- 4. install --------------------------------------------------------------

mkdir -p "$INSTALL_DIR"
install -m 755 "${tmp}/${BIN}-${VERSION}-${target}/${BIN}" "${INSTALL_DIR}/${BIN}" 2>/dev/null \
    || { cp "${tmp}/${BIN}-${VERSION}-${target}/${BIN}" "${INSTALL_DIR}/${BIN}" && chmod 755 "${INSTALL_DIR}/${BIN}"; }

say "==> installed $("${INSTALL_DIR}/${BIN}" --version) to ${INSTALL_DIR}/${BIN}"

# --- 5. what to do next ------------------------------------------------------

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        say ""
        say "${INSTALL_DIR} is not on your PATH. Add this to your shell profile:"
        say "    export PATH=\"${INSTALL_DIR}:\$PATH\""
        ;;
esac

if [ -z "${HERDR_PANE_ID:-}" ]; then
    say ""
    say "grove draws its previews through herdr's pane graphics API, so run it"
    say "in a herdr pane — outside one you get the tree and text, but no pictures."
fi
