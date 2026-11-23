#!/bin/sh
# Soli installer — works with any POSIX shell
# Usage: curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh
#   or:  sh install.sh [--system]

set -e

REPO="solisoft/soli_lang"
SYSTEM_DIR="/usr/local/bin"
SYSTEM_INSTALL=0
USER_INSTALL=0

for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM_INSTALL=1 ;;
    --user)   USER_INSTALL=1 ;;
    --help|-h)
      echo "Usage: install.sh [--system | --user]"
      echo "  --system  Install to ${SYSTEM_DIR} (requires sudo when not root)"
      echo "  --user    Install to ~/.local/bin (even when running as root)"
      echo "  Default:  ~/.local/bin, or ${SYSTEM_DIR} when running as root"
      exit 0
      ;;
    *) echo "Unknown option: $arg"; exit 1 ;;
  esac
done

# --- Decide install location ---
# Running as root (e.g. `curl ... | sudo sh`, Docker build) installs globally
# so every user can run `soli`. A normal user installs to ~/.local/bin. The
# --system / --user flags force a choice explicitly.
IS_ROOT=0
[ "$(id -u)" = "0" ] && IS_ROOT=1

if [ "$USER_INSTALL" = "1" ]; then
  INSTALL_DIR="$HOME/.local/bin"
elif [ "$SYSTEM_INSTALL" = "1" ] || [ "$IS_ROOT" = "1" ]; then
  INSTALL_DIR="$SYSTEM_DIR"
else
  INSTALL_DIR="$HOME/.local/bin"
fi

# Elevate with sudo only for a system dir when we are not already root.
NEED_ELEVATION=0
if [ "$INSTALL_DIR" = "$SYSTEM_DIR" ] && [ "$IS_ROOT" != "1" ]; then
  NEED_ELEVATION=1
fi

# --- Detect OS ---
OS="$(uname -s)"
case "$OS" in
  Linux*)  OS="linux" ;;
  Darwin*) OS="darwin" ;;
  *) echo "Error: unsupported operating system: $OS"; exit 1 ;;
esac

# --- Detect architecture ---
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)   ARCH="amd64" ;;
  aarch64|arm64)   ARCH="arm64" ;;
  *) echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
esac

echo "Detected platform: ${OS}-${ARCH}"

# --- Pick a download tool ---
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO- "$1"; }
else
  echo "Error: curl or wget is required"; exit 1
fi

# --- What is installed now ---
# Asked of the binary itself: `soli --version` has always been safe to run.
# The copy in INSTALL_DIR is the one this replaces; a different `soli` first
# on the PATH is the one a shell will actually run, which is worth knowing
# before and after.
OLD_VERSION=""
if [ -x "${INSTALL_DIR}/soli" ]; then
  OLD_VERSION=$("${INSTALL_DIR}/soli" --version 2>/dev/null | head -1)
  echo "Installed now: ${OLD_VERSION:-unknown} (${INSTALL_DIR}/soli)"
fi

# --- Get latest version tag ---
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
TAG=""
if TAG=$(fetch "$API_URL" 2>/dev/null | grep '"tag_name"' | head -1 | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/'); then
  if [ -z "$TAG" ]; then
    TAG=""
  fi
fi

# The API is rate-limited (60 calls an hour for an address without a token);
# the `releases/latest` page redirects to the tag and is not. Where the tag
# comes from does not matter to its trust: a signed release names its tag.
# A tag is a version: anything else in it is not one this script follows
# (checked here, so that the redirect is asked next, and again after it).
case "$TAG" in
  *[!A-Za-z0-9._-]*) TAG="" ;;
esac
if [ -z "$TAG" ]; then
  LATEST_URL="https://github.com/${REPO}/releases/latest"
  if command -v curl >/dev/null 2>&1; then
    RESOLVED=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$LATEST_URL" 2>/dev/null || true)
  else
    # The header as the server sent it (wget also prints its own
    # "Location: … [following]" line): the first word after the name.
    RESOLVED=$(wget -S --spider "$LATEST_URL" 2>&1 | sed -n 's/^ *[Ll]ocation: *\([^ ]*\).*/\1/p' | tr -d '\r' | tail -1)
  fi
  case "$RESOLVED" in
    */releases/tag/v[0-9]*) TAG="${RESOLVED##*/releases/tag/}" ;;
  esac
fi

case "$TAG" in
  *[!A-Za-z0-9._-]*) TAG="" ;;
esac

# --- Never backwards ---
# "Latest" is what a server said. A release older than the one installed is
# not an update, whoever says so (SOLI_INSTALL_ALLOW_DOWNGRADE=1 to do it on
# purpose).
version_number() {
  # "soli 2.41.0", "v2.41.0" -> 2041000; anything else -> nothing.
  printf '%s\n' "$1" | sed -n 's/^[^0-9]*0*\([0-9]\{1,4\}\)\.0*\([0-9]\{1,3\}\)\.0*\([0-9]\{1,3\}\).*$/\1 \2 \3/p' |
    { read -r major minor patch || exit 0; echo $((major * 1000000 + minor * 1000 + patch)); }
}
if [ -n "$TAG" ] && [ -n "$OLD_VERSION" ] && [ "${SOLI_INSTALL_ALLOW_DOWNGRADE:-}" != "1" ]; then
  OLD_NUMBER=$(version_number "$OLD_VERSION")
  NEW_NUMBER=$(version_number "$TAG")
  if [ -n "$OLD_NUMBER" ] && [ -n "$NEW_NUMBER" ] && [ "$NEW_NUMBER" -lt "$OLD_NUMBER" ]; then
    echo "Error: ${TAG} is older than the installed ${OLD_VERSION} — not installing it."
    echo "Set SOLI_INSTALL_ALLOW_DOWNGRADE=1 to go back on purpose."
    exit 1
  fi
fi

# --- Download and extract ---
TARBALL="soli-${OS}-${ARCH}.tar.gz"
if [ -n "$TAG" ]; then
  echo "Installing Soli ${TAG} ..."
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${TARBALL}"
else
  # The API is rate-limited (60 calls an hour without a token), and this used
  # to fall back to v0.20.0 -- installing a release years old behind a
  # one-line warning. GitHub's `latest/download` link needs no API, and the
  # binary says which version it is once it is here.
  echo "Could not ask GitHub which release is latest; downloading it without asking ..."
  DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${TARBALL}"
fi
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${DOWNLOAD_URL} ..."
fetch "$DOWNLOAD_URL" > "${TMP_DIR}/${TARBALL}"

# --- Verify it ---
# Every release publishes `<tarball>.sha256` next to the tarball. The
# download used to be unpacked and run as it came; it is now checked against
# that digest, and an installation with no digest to check against stops
# here (SOLI_INSTALL_SKIP_CHECKSUM=1 installs it anyway).
if command -v sha256sum >/dev/null 2>&1; then
  digest_of() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  digest_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  digest_of() { echo ""; }
fi
EXPECTED=$(fetch "${DOWNLOAD_URL}.sha256" 2>/dev/null | cut -d' ' -f1 | tr 'A-F' 'a-f')
ACTUAL=$(digest_of "${TMP_DIR}/${TARBALL}")
if [ -n "$EXPECTED" ] && [ -n "$ACTUAL" ]; then
  if [ "$EXPECTED" != "$ACTUAL" ]; then
    echo "Error: checksum mismatch for ${TARBALL}"
    echo "  expected ${EXPECTED}"
    echo "  got      ${ACTUAL}"
    exit 1
  fi
  echo "Checksum verified."
elif [ "${SOLI_INSTALL_SKIP_CHECKSUM:-}" = "1" ]; then
  echo "Warning: the download could not be verified (no .sha256, or no sha256sum/shasum); installing anyway."
else
  echo "Error: the download could not be verified:"
  [ -z "$EXPECTED" ] && echo "  no ${TARBALL}.sha256 could be fetched"
  [ -z "$ACTUAL" ] && echo "  neither sha256sum nor shasum is installed"
  echo "Set SOLI_INSTALL_SKIP_CHECKSUM=1 to install without verification."
  exit 1
fi

# --- Verify its signature ---
# The digest above sits beside the tarball: whoever can replace one can
# replace the other. A release is also signed — ECDSA P-256 over its tag, the
# artifact's name and its digest, one per line — with a key that is not on
# the release host. RELEASE_PUBKEY is the public half (base64, as
# `soli update-keygen` prints it), written on the line below when the key is
# made; while it is empty, releases are not signed yet and the digest is all
# there is to check. SOLI_RELEASE_PUBKEY names another key (a mirror's).
RELEASE_PUBKEY=""
RELEASE_PUBKEY="${SOLI_RELEASE_PUBKEY:-$RELEASE_PUBKEY}"
if [ -n "$RELEASE_PUBKEY" ]; then
  if ! command -v openssl >/dev/null 2>&1; then
    echo "Error: openssl is needed to verify the release signature."
    exit 1
  fi
  if [ -z "$TAG" ]; then
    echo "Error: the release's version is not known (GitHub did not say which is latest),"
    echo "  and the signature names it. Try again in a moment."
    exit 1
  fi
  if ! fetch "${DOWNLOAD_URL}.sig" > "${TMP_DIR}/sig.b64" 2>/dev/null || [ ! -s "${TMP_DIR}/sig.b64" ]; then
    echo "Error: ${TAG} publishes no signature for ${TARBALL} — not installing it."
    exit 1
  fi
  # The key as a SubjectPublicKeyInfo: the fixed P-256 header, then the point.
  {
    printf '\060\131\060\023\006\007\052\206\110\316\075\002\001\006\010\052\206\110\316\075\003\001\007\003\102\000'
    printf '%s' "$RELEASE_PUBKEY" | tr -d '[:space:]' | openssl base64 -d -A 2>/dev/null || true
  } > "${TMP_DIR}/key.der"
  if [ "$(wc -c < "${TMP_DIR}/key.der" | tr -d ' ')" != "91" ]; then
    echo "Error: RELEASE_PUBKEY is not a P-256 public key (base64, as \`soli update-keygen\` prints it)."
    exit 1
  fi
  printf '%s\n%s\n%s\n' "$TAG" "$TARBALL" "$ACTUAL" > "${TMP_DIR}/statement"
  VERDICT=""
  if openssl base64 -d -A < "${TMP_DIR}/sig.b64" > "${TMP_DIR}/sig.der" 2>/dev/null; then
    VERDICT=$(openssl dgst -sha256 -verify "${TMP_DIR}/key.der" -keyform DER \
      -signature "${TMP_DIR}/sig.der" "${TMP_DIR}/statement" 2>/dev/null || true)
  fi
  # The words, not only the exit status.
  if [ "$VERDICT" = "Verified OK" ]; then
    echo "Signature verified."
  else
    echo "Error: the signature of ${TARBALL} is not the release key's for ${TAG} — not installing it."
    exit 1
  fi
fi

tar xzf "${TMP_DIR}/${TARBALL}" -C "$TMP_DIR"

NEW_VERSION=$("${TMP_DIR}/soli" --version 2>/dev/null | head -1)
echo "Downloaded: ${NEW_VERSION:-a binary that did not say its version}"

# --- Install binary ---
if [ "$NEED_ELEVATION" = "1" ]; then
  echo "Installing to ${INSTALL_DIR} (requires sudo) ..."
  sudo install -m 755 "${TMP_DIR}/soli" "${INSTALL_DIR}/soli"
else
  echo "Installing to ${INSTALL_DIR} ..."
  mkdir -p "$INSTALL_DIR"
  install -m 755 "${TMP_DIR}/soli" "${INSTALL_DIR}/soli"
fi

# --- Clean up stale per-user copies on a global install ---
# Older versions of this script installed to ~/.local/bin even when run as
# root, leaving /root/.local/bin/soli (or $HOME/.local/bin/soli) behind. When
# we install globally, remove those stale copies so PATH can't shadow the new
# binary with an outdated one.
if [ "$INSTALL_DIR" = "$SYSTEM_DIR" ]; then
  for stale in "/root/.local/bin/soli" "${HOME}/.local/bin/soli"; do
    if [ "$stale" != "${INSTALL_DIR}/soli" ] && [ -e "$stale" ]; then
      echo "Removing stale install: ${stale}"
      rm -f "$stale"
    fi
  done
fi

# --- Check PATH ---
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo ""
    echo "WARNING: ${INSTALL_DIR} is not in your PATH."
    echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
    echo ""
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
    ;;
esac

# --- Verify ---
# The version is read from the file just installed, not from whatever `soli`
# the PATH finds first -- which may be another copy, and used to be what
# this printed as the result.
echo ""
echo "Soli installed to ${INSTALL_DIR}/soli"
echo "  version: $("${INSTALL_DIR}/soli" --version 2>/dev/null | head -1)"
if [ -n "$OLD_VERSION" ]; then
  if [ "$OLD_VERSION" = "$NEW_VERSION" ]; then
    echo "  (the version that was already installed)"
  else
    echo "  was:     ${OLD_VERSION}"
  fi
fi
FOUND=$(command -v soli 2>/dev/null || true)
if [ -n "$FOUND" ] && [ "$FOUND" != "${INSTALL_DIR}/soli" ]; then
  echo ""
  echo "Note: your shell runs ${FOUND} first, which is $("$FOUND" --version 2>/dev/null | head -1)."
  echo "Put ${INSTALL_DIR} earlier on your PATH, or remove that copy, to run this one."
fi
