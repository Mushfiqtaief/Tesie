#!/usr/bin/env bash
set -euo pipefail

REPO="${TESIE_REPO:-Mushfiqtaief/Tesie}"
INSTALL_DIR="${TESIE_INSTALL_DIR:-$HOME/.local/share/tesie}"
VERSION="${TESIE_VERSION:-latest}"
NO_OPEN="${TESIE_NO_OPEN:-0}"
FROM_SOURCE="${TESIE_FROM_SOURCE:-0}"

bold() { printf "\033[1m%s\033[0m\n" "$*"; }
info() { printf "  \033[36m•\033[0m %s\n" "$*"; }
ok()   { printf "  \033[32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[33m!\033[0m %s\n" "$*" >&2; }
fail() { printf "  \033[31m✗\033[0m %s\n" "$*" >&2; exit 1; }

UNAME_S="$(uname -s)"
case "$UNAME_S" in
  Darwin) OS_TAG="macos" ;;
  Linux)  OS_TAG="linux" ;;
  *)      fail "Unsupported OS '$UNAME_S'. Use scripts/install.ps1 on Windows." ;;
esac

ARCH_RAW="$(uname -m)"
if [ "$OS_TAG" = "macos" ] && [ "$ARCH_RAW" = "x86_64" ]; then
  if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
    ARCH_RAW="arm64"
  fi
fi

case "$ARCH_RAW" in
  arm64|aarch64) ARCH_TAG="arm64" ;;
  x86_64|amd64)  ARCH_TAG="x64" ;;
  *)             fail "Unsupported CPU architecture '$ARCH_RAW'." ;;
esac

ARCHIVE_NAME="tesie-${OS_TAG}-${ARCH_TAG}.tar.gz"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tesie-install.XXXXXX")"
STAGE_DIR="$TMP_DIR/stage"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/www"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

fetch_file() {
  local url="$1"
  local dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 2 --connect-timeout 15 -o "$dest" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    return 1
  fi
}

verify_checksum() {
  local sums_file="$1"
  local target_file="$2"
  local filename
  filename="$(basename "$target_file")"
  local expected
  expected="$(awk -v f="$filename" '$2 == f || $2 == ("*" f) { print $1; exit }' "$sums_file" || true)"
  [ -n "$expected" ] || return 0

  local actual=""
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$target_file" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$target_file" | awk '{print $1}')"
  else
    return 0
  fi

  if [ "$actual" != "$expected" ]; then
    fail "SHA256 mismatch for $filename (expected $expected, got $actual)"
  fi
  ok "Verified SHA256 checksum ($filename)"
}

build_from_source() {
  local src_dir="$1"
  bold "Building Tesie from source ($src_dir)..."

  local missing=""
  command -v go >/dev/null 2>&1    || missing="$missing go"
  command -v cargo >/dev/null 2>&1 || missing="$missing rust/cargo"
  if ! command -v bun >/dev/null 2>&1 && ! command -v npm >/dev/null 2>&1; then
    missing="$missing bun-or-nodejs"
  fi
  if [ -n "$missing" ]; then
    warn "Missing build tools:$missing"
    case "$OS_TAG" in
      macos)
        warn "Install via Homebrew: brew install go rust oven-sh/bun/bun"
        ;;
      linux)
        warn "Ubuntu/Debian: sudo apt update && sudo apt install -y build-essential curl git golang-go nodejs npm && curl -fsSL https://sh.rustup.rs | sh"
        warn "Fedora/RHEL:   sudo dnf install -y gcc gcc-c++ make git golang nodejs npm && curl -fsSL https://sh.rustup.rs | sh"
        warn "Arch Linux:    sudo pacman -S --needed base-devel git go rustup nodejs npm && rustup default stable"
        warn "Alpine Linux:  sudo apk add build-base git go rust cargo nodejs npm"
        ;;
    esac
    fail "Install the missing toolchains above and re-run."
  fi

  info "Building TesieCore (Go network engine)..."
  (
    cd "$src_dir/mihomo"
    CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o "$STAGE_DIR/bin/tesie-core" .
  )

  info "Building Tesie dashboard SPA..."
  (
    cd "$src_dir/frontend"
    if command -v bun >/dev/null 2>&1; then
      bun install
      bun run build
    else
      npm install
      npm run build
    fi
    cp -R dist/. "$STAGE_DIR/www/"
  )

  info "Building Tesie gateway & daemon (Rust release)..."
  (
    cd "$src_dir/backend"
    cargo build --release --bins
    cp "target/release/tesie" "$STAGE_DIR/bin/tesie"
    cp "target/release/tesie-daemon" "$STAGE_DIR/bin/tesie-daemon"
  )
}

bold "Installing Tesie ($OS_TAG/$ARCH_TAG)..."

INSTALLED_VIA_ARCHIVE=0
if [ "$FROM_SOURCE" != "1" ] && [ "$REPO" != "OWNER/Tesie" ]; then
  if [ "$VERSION" = "latest" ]; then
    BASE_URL="https://github.com/${REPO}/releases/latest/download"
  else
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
  fi

  info "Checking prebuilt release: $BASE_URL/$ARCHIVE_NAME"
  if fetch_file "$BASE_URL/$ARCHIVE_NAME" "$TMP_DIR/$ARCHIVE_NAME" 2>/dev/null; then
    if fetch_file "$BASE_URL/SHA256SUMS.txt" "$TMP_DIR/SHA256SUMS.txt" 2>/dev/null; then
      verify_checksum "$TMP_DIR/SHA256SUMS.txt" "$TMP_DIR/$ARCHIVE_NAME"
    fi
    tar -xzf "$TMP_DIR/$ARCHIVE_NAME" -C "$STAGE_DIR"
    INSTALLED_VIA_ARCHIVE=1
    ok "Downloaded prebuilt $ARCHIVE_NAME"
  else
    warn "Prebuilt release '$ARCHIVE_NAME' not found yet; falling back to source build."
  fi
fi

if [ "$INSTALLED_VIA_ARCHIVE" -eq 0 ]; then
  if [ -f "backend/Cargo.toml" ] && [ -f "mihomo/go.mod" ]; then
    build_from_source "$(pwd)"
  elif [ "$REPO" != "OWNER/Tesie" ]; then
    command -v git >/dev/null 2>&1 || fail "git is required to clone https://github.com/${REPO}.git"
    info "Cloning https://github.com/${REPO}.git..."
    git clone --depth 1 "https://github.com/${REPO}.git" "$TMP_DIR/src"
    build_from_source "$TMP_DIR/src"
  else
    fail "Set TESIE_REPO=<username>/Tesie or run this script from a cloned Tesie repository."
  fi
fi

[ -f "$STAGE_DIR/bin/tesie" ] || fail "Staged archive missing bin/tesie"
[ -f "$STAGE_DIR/bin/tesie-daemon" ] || fail "Staged archive missing bin/tesie-daemon"
[ -f "$STAGE_DIR/bin/tesie-core" ] || fail "Staged archive missing bin/tesie-core"

if [ -x "$INSTALL_DIR/bin/tesie" ]; then
  info "Stopping existing Tesie daemon cleanly..."
  "$INSTALL_DIR/bin/tesie" stop >/dev/null 2>&1 || true
fi

mkdir -p "$INSTALL_DIR/bin" "$INSTALL_DIR/www"
rm -f "$INSTALL_DIR/bin/tesie" "$INSTALL_DIR/bin/tesie-daemon" "$INSTALL_DIR/bin/tesie-core"
cp -f "$STAGE_DIR/bin/tesie" "$INSTALL_DIR/bin/tesie"
cp -f "$STAGE_DIR/bin/tesie-daemon" "$INSTALL_DIR/bin/tesie-daemon"
cp -f "$STAGE_DIR/bin/tesie-core" "$INSTALL_DIR/bin/tesie-core"
chmod +x "$INSTALL_DIR/bin/tesie" "$INSTALL_DIR/bin/tesie-daemon" "$INSTALL_DIR/bin/tesie-core"

if [ -d "$STAGE_DIR/www" ]; then
  rm -rf "$INSTALL_DIR/www"
  mkdir -p "$INSTALL_DIR/www"
  cp -R "$STAGE_DIR/www/." "$INSTALL_DIR/www/"
fi

if [ "$OS_TAG" = "macos" ] && command -v xattr >/dev/null 2>&1; then
  xattr -cr "$INSTALL_DIR" 2>/dev/null || true
  ok "Cleared macOS quarantine attributes on $INSTALL_DIR"
fi

ok "Installed binaries into $INSTALL_DIR"

info "Registering global CLI (~/.local/bin/tesie)..."
"$INSTALL_DIR/bin/tesie" install

info "Starting Tesie background daemon..."
if [ "$NO_OPEN" = "1" ]; then
  "$INSTALL_DIR/bin/tesie" start
else
  "$INSTALL_DIR/bin/tesie" start --open
fi

printf "\n"
ok "Tesie is ready! Run 'tesie status' anytime or visit http://127.0.0.1:20129"
