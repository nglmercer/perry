#!/bin/sh
# Perry installer — downloads the latest release from GitHub
# Usage: curl -fsSL https://perryts.com/install.sh | sh

set -eu

REPO="PerryTS/perry"
INSTALL_DIR="/usr/local/bin"
LIB_DIR="/usr/local/lib"

# Only show progress bars on interactive terminals
if [ -t 2 ]; then
  CURL_PROGRESS="--progress-bar"
  PV=""
  if command -v pv >/dev/null 2>&1; then
    PV="pv"
  fi
else
  CURL_PROGRESS="-fsS"
  PV=""
fi

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) OS="macos" ;;
  linux)  OS="linux" ;;
  *)
    echo "Error: Unsupported OS: $OS"
    echo "See https://github.com/$REPO for manual install instructions."
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "Error: Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

ARTIFACT="perry-${OS}-${ARCH}.tar.gz"

echo "Detecting platform: ${OS}/${ARCH}"

# Find the most recent release that actually has our platform asset.
echo "Locating most recent release with $ARTIFACT..."

TAGS=$(
  curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=30" \
    | grep '"tag_name":' \
    | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
)

LATEST=""
for tag in $TAGS; do
  url="https://github.com/$REPO/releases/download/$tag/$ARTIFACT"
  status=$(curl -fsSLI -o /dev/null -w "%{http_code}" "$url" 2>/dev/null || echo "000")
  final="${status#"${status%???}"}"
  if [ "$final" = "200" ]; then
    LATEST="$tag"
    break
  fi
done

if [ -z "$LATEST" ]; then
  echo "Error: No recent release has $ARTIFACT attached."
  echo "       Releases list: https://github.com/$REPO/releases"
  echo "       Try: npm install -g @perryts/perry  (always works)"
  exit 1
fi

echo "Using version: $LATEST"

URL="https://github.com/$REPO/releases/download/$LATEST/$ARTIFACT"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading $ARTIFACT..."
START=$(date +%s 2>/dev/null || echo 0)
curl -L $CURL_PROGRESS -o "$TMPDIR/perry.tar.gz" "$URL"
END=$(date +%s 2>/dev/null || echo 0)

if [ "$END" -gt "$START" ] && [ "$START" -gt 0 ]; then
  echo "  Done in $((END - START))s ($(ls -lh "$TMPDIR/perry.tar.gz" | awk '{print $5}'))"
fi

echo "Extracting..."
if [ -n "$PV" ]; then
  pv "$TMPDIR/perry.tar.gz" | tar xz -C "$TMPDIR"
else
  tar xzf "$TMPDIR/perry.tar.gz" -C "$TMPDIR"
fi

# Install binary
if [ -w "$INSTALL_DIR" ]; then
  cp "$TMPDIR/perry" "$INSTALL_DIR/perry"
  chmod 755 "$INSTALL_DIR/perry"
  for lib in "$TMPDIR"/libperry_*.a; do
    [ -f "$lib" ] && cp "$lib" "$LIB_DIR/"
  done
else
  echo "Installing to $INSTALL_DIR (requires sudo)..."
  sudo cp "$TMPDIR/perry" "$INSTALL_DIR/perry"
  sudo chmod 755 "$INSTALL_DIR/perry"
  for lib in "$TMPDIR"/libperry_*.a; do
    [ -f "$lib" ] && sudo cp "$lib" "$LIB_DIR/"
  done
fi

echo ""
echo "Perry $LATEST installed successfully!"
echo ""
echo "Quick start:"
echo "  echo 'console.log(\"hello\")' > hello.ts"
echo "  perry hello.ts -o hello && ./hello"
echo ""
echo "Run 'perry doctor' to verify your setup."
