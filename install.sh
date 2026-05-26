#!/usr/bin/env bash
# One-command installer for the watch-tool patched codex.
#
#   curl -fsSL https://raw.githubusercontent.com/wolverin0/codex/feat/watch-monitor-0133/install.sh | bash
#
# What this does:
#   1. Detects platform (currently supports linux x86_64 only).
#   2. Verifies official @openai/codex@0.133.0 is installed (or installs it).
#   3. Downloads our pre-built patched binary from this repo's GitHub Release.
#   4. Backs up the stock binary and swaps in the patched one.
#   5. Verifies the swap with --version + a strings probe.
#
# The npm shim, version checks, and SQLite state DB all stay the same —
# we ONLY replace the platform-specific binary inside the npm vendored
# slot. To revert: cp the .orig-* backup back. See WATCH-TOOL.md.

set -euo pipefail

REPO="wolverin0/codex"
TAG="v0.133.0-watch.1"
CODEX_VERSION="0.133.0"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS-$ARCH" in
  Linux-x86_64)
    ASSET="codex-watch-${CODEX_VERSION}-linux-x64"
    VENDOR_PKG="@openai/codex-linux-x64"
    VENDOR_TRIPLE="x86_64-unknown-linux-musl"
    ;;
  *)
    echo "✗ Unsupported platform: $OS-$ARCH"
    echo "  Currently only Linux x86_64 (incl. WSL Ubuntu) has a pre-built binary."
    echo "  For Mac/Windows: clone the repo and build from source — see WATCH-TOOL.md."
    exit 1
    ;;
esac

echo "==> codex watch-tool installer"
echo "    platform:        $OS-$ARCH"
echo "    release tag:     $TAG"
echo "    asset:           $ASSET"
echo ""

# 1. Ensure official codex installed at matching version
if ! command -v codex >/dev/null 2>&1; then
  echo "==> Official @openai/codex is not installed."
  echo "    Run this from a shell with npm available:"
  echo "      npm install -g @openai/codex@${CODEX_VERSION}"
  echo "    Then re-run this installer."
  exit 1
fi

INSTALLED_VER="$(codex --version 2>/dev/null | awk '{print $NF}' | sed 's/^v//')"
if [[ "$INSTALLED_VER" != "$CODEX_VERSION" ]]; then
  echo "✗ Version mismatch: official codex is $INSTALLED_VER, this patch is for $CODEX_VERSION."
  echo ""
  echo "  The SQLite state DB schema is version-coupled. Mixed versions cause"
  echo "  'no such table: thread_goals' (and similar). Align first:"
  echo "    npm install -g @openai/codex@${CODEX_VERSION}"
  echo ""
  echo "  Or follow the per-release update procedure in codex-watcher to"
  echo "  re-port this patch onto whatever version you have installed:"
  echo "    https://github.com/wolverin0/codex-watcher"
  exit 1
fi

echo "==> Detected codex $INSTALLED_VER (version match)"

# 2. Locate the npm-installed vendor slot
NPM_ROOT="$(npm root -g 2>/dev/null || true)"
if [[ -z "$NPM_ROOT" ]]; then
  echo "✗ Could not run 'npm root -g'. Is npm on PATH?"
  exit 1
fi

VENDOR_SLOT="$NPM_ROOT/@openai/codex/node_modules/$VENDOR_PKG/vendor/$VENDOR_TRIPLE/codex/codex"
if [[ ! -f "$VENDOR_SLOT" ]]; then
  echo "✗ Vendor slot not found: $VENDOR_SLOT"
  echo "  Your codex install may have a different layout. Check WATCH-TOOL.md"
  echo "  for the manual swap procedure."
  exit 1
fi

# 3. Check no codex is currently running (Linux holds an exclusive lock on running .exes)
if pgrep -f "$VENDOR_SLOT" >/dev/null 2>&1; then
  echo "✗ A codex process is currently running from this vendor slot."
  echo "  Close all codex TUI sessions and re-run this installer."
  echo "  Running PIDs:"
  pgrep -af "$VENDOR_SLOT" | sed 's/^/    /'
  exit 1
fi

# 4. Download the patched binary into a temp file
TMP="$(mktemp -t codex-watch-XXXXXX)"
trap 'rm -f "$TMP"' EXIT

ASSET_URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
echo "==> Downloading patched binary..."
echo "    from: $ASSET_URL"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL --progress-bar "$ASSET_URL" -o "$TMP"
elif command -v wget >/dev/null 2>&1; then
  wget -q --show-progress -O "$TMP" "$ASSET_URL"
else
  echo "✗ Need either curl or wget."
  exit 1
fi

# Sanity-check size (the binary should be > 100MB; a 404 HTML page would be tiny).
SIZE="$(stat -c%s "$TMP" 2>/dev/null || stat -f%z "$TMP")"
if [[ "$SIZE" -lt 50000000 ]]; then
  echo "✗ Downloaded asset is only ${SIZE} bytes — likely a 404 page, not the binary."
  echo "  Check that release $TAG exists at: https://github.com/${REPO}/releases"
  exit 1
fi

chmod +x "$TMP"

# 5. Verify the patched binary works + has our features
if ! "$TMP" --version >/dev/null 2>&1; then
  echo "✗ Downloaded binary does not run. Aborting swap."
  exit 1
fi

if ! strings "$TMP" 2>/dev/null | grep -q "watch auto-stopped after"; then
  echo "✗ Downloaded binary missing 'watch auto-stopped after' string — features not baked in."
  echo "  This shouldn't happen for a release tag. Open an issue at https://github.com/${REPO}/issues"
  exit 1
fi
echo "==> Patched binary verified (--version OK, feature strings present)."

# 6. Back up stock binary and swap
BACKUP="${VENDOR_SLOT}.orig-$(date +%s)"
cp -p "$VENDOR_SLOT" "$BACKUP"
echo "==> Backed up stock binary to: $BACKUP"

cp "$TMP" "$VENDOR_SLOT"
echo "==> Swapped patched binary into: $VENDOR_SLOT"

# 7. Final verification
NEW_VER="$(codex --version 2>/dev/null | awk '{print $NF}' | sed 's/^v//')"
echo ""
echo "✓ Install complete."
echo "    codex --version → $NEW_VER (patched with watch tool)"
echo ""
echo "Try it: in a new codex session, ask:"
echo "    Register a watch with command 'cat README.md', instruction"
echo "    'reply CHANGED with the diff', interval 5 seconds."
echo ""
echo "You should see the footer chip '· watch: cat README.md' appear."
echo ""
echo "To revert to stock codex:"
echo "    cp \"$BACKUP\" \"$VENDOR_SLOT\""
echo ""
echo "Maintenance kit (per-release re-port automation):"
echo "    https://github.com/wolverin0/codex-watcher"
