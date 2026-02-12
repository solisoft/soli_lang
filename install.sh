#!/bin/sh
# Soli installer — works with any POSIX shell
# Usage: curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh
#   or:  sh install.sh [--system]

set -e

REPO="solisoft/soli_lang"
INSTALL_DIR="$HOME/.local/bin"
SYSTEM_INSTALL=0

for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM_INSTALL=1; INSTALL_DIR="/usr/local/bin" ;;
    --help|-h)
      echo "Usage: install.sh [--system]"
      echo "  --system  Install to /usr/local/bin (requires sudo)"
      echo "  Default:  Install to ~/.local/bin"
      exit 0
      ;;
    *) echo "Unknown option: $arg"; exit 1 ;;
  esac
done

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

# --- Get latest version tag ---
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
TAG=""
if TAG=$(fetch "$API_URL" 2>/dev/null | grep '"tag_name"' | head -1 | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/'); then
  if [ -z "$TAG" ]; then
    TAG=""
  fi
fi

if [ -z "$TAG" ]; then
  echo "Warning: could not fetch latest release, falling back to v0.20.0"
  TAG="v0.20.0"
fi

echo "Installing Soli ${TAG} ..."

# --- Download and extract ---
TARBALL="soli-${OS}-${ARCH}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${TARBALL}"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${DOWNLOAD_URL} ..."
fetch "$DOWNLOAD_URL" > "${TMP_DIR}/${TARBALL}"

tar xzf "${TMP_DIR}/${TARBALL}" -C "$TMP_DIR"

# --- Install binary ---
if [ "$SYSTEM_INSTALL" = "1" ]; then
  echo "Installing to ${INSTALL_DIR} (may require sudo) ..."
  sudo install -m 755 "${TMP_DIR}/soli" "${INSTALL_DIR}/soli"
else
  mkdir -p "$INSTALL_DIR"
  install -m 755 "${TMP_DIR}/soli" "${INSTALL_DIR}/soli"
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
if command -v soli >/dev/null 2>&1; then
  echo ""
  echo "Soli installed successfully!"
  soli --version
else
  echo ""
  echo "Soli installed to ${INSTALL_DIR}/soli"
  echo "Run 'soli --version' to verify (you may need to reload your shell)."
fi
