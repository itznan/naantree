#!/bin/sh
set -eu

# Naantree Installer
# Zero-dependency POSIX shell installer for Linux and macOS

REPO="itznan/naantree"
VERSION="${NAANTREE_VERSION:-latest}"

# Detect OS
OS_RAW="$(uname -s)"
case "$OS_RAW" in
  Linux)
    OS="linux"
    ;;
  Darwin)
    OS="darwin"
    ;;
  *)
    echo "Error: Unsupported operating system: $OS_RAW" >&2
    echo "Naantree currently provides pre-built binaries for Linux and macOS (Darwin)." >&2
    exit 1
    ;;
esac

# Detect Architecture
ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  arm64|aarch64)
    ARCH="aarch64"
    ;;
  *)
    echo "Error: Unsupported CPU architecture: $ARCH_RAW" >&2
    echo "Naantree supports x86_64 and arm64/aarch64." >&2
    exit 1
    ;;
esac

# Determine installation directory: /usr/local/bin if writable, otherwise ~/.local/bin
if [ -w "/usr/local/bin" ]; then
  INSTALL_DIR="/usr/local/bin"
elif [ ! -e "/usr/local/bin" ] && [ -w "/usr/local" ]; then
  mkdir -p "/usr/local/bin"
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="${HOME}/.local/bin"
  mkdir -p "$INSTALL_DIR"
fi

# Download helper function (curl or wget)
download() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    echo "Error: curl or wget is required to download naantree." >&2
    exit 1
  fi
}

# Temporary directory with automatic cleanup
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'naantree-install')"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM HUP

# Determine release asset names to try
ASSET_NAME="naantree-${OS}-${ARCH}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

ARCHIVE="${TMP_DIR}/${ASSET_NAME}"
DOWNLOAD_URL="${BASE_URL}/${ASSET_NAME}"

echo "Fetching naantree release for ${OS}-${ARCH}..."
if ! download "$DOWNLOAD_URL" "$ARCHIVE" 2>/dev/null; then
  # Try arm64 fallback if aarch64 failed on Darwin
  if [ "$OS" = "darwin" ] && [ "$ARCH" = "aarch64" ]; then
    FALLBACK_ASSET="naantree-darwin-arm64.tar.gz"
    DOWNLOAD_URL="${BASE_URL}/${FALLBACK_ASSET}"
    ARCHIVE="${TMP_DIR}/${FALLBACK_ASSET}"
    if ! download "$DOWNLOAD_URL" "$ARCHIVE" 2>/dev/null; then
      echo "Error: Failed to download naantree from $DOWNLOAD_URL" >&2
      exit 1
    fi
  else
    echo "Error: Failed to download naantree from $DOWNLOAD_URL" >&2
    exit 1
  fi
fi

# Extract binary
echo "Extracting binary..."
tar -xzf "$ARCHIVE" -C "$TMP_DIR"

if [ ! -f "${TMP_DIR}/naantree" ]; then
  echo "Error: Binary 'naantree' not found in downloaded archive." >&2
  exit 1
fi

# Install binary
TARGET_PATH="${INSTALL_DIR}/naantree"
echo "Installing to ${TARGET_PATH}..."
cp "${TMP_DIR}/naantree" "$TARGET_PATH"
chmod 755 "$TARGET_PATH"

# Verify installation
if [ -x "$TARGET_PATH" ]; then
  VERSION_INFO="$("$TARGET_PATH" --version 2>&1 || true)"
  echo ""
  echo "==> Installation successful!"
  echo "Binary location: $TARGET_PATH"
  if [ -n "$VERSION_INFO" ]; then
    echo "Version:         $VERSION_INFO"
  fi
else
  echo "Error: Installation verification failed. $TARGET_PATH is not executable." >&2
  exit 1
fi

# Check PATH
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    ;;
  *)
    echo ""
    echo "Note: '${INSTALL_DIR}' is not in your current PATH."
    echo "Add it by adding this line to your ~/.bashrc, ~/.zshrc, or profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac
