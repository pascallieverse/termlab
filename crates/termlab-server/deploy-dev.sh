#!/usr/bin/env bash
# Puts the termlab-server from this checkout (uncommitted changes included) on a server
# that runs it as the systemd unit `termlab-server` (see docs/self-hosting.md), so changes
# can be tried there without a release.
#
#   crates/termlab-server/deploy-dev.sh             # build, upload, restart
#   crates/termlab-server/deploy-dev.sh --rollback  # put the previous binary back
#
# Where to: TERMLAB_DEPLOY_HOST (user@host, with passwordless sudo) and optionally
# TERMLAB_DEPLOY_PORT, from the environment or from `.deploy-server.env` at the repository
# root (git-ignored).
#
# Builds for x86-64 Linux with the x86_64-linux-gnu cross toolchain from Homebrew
# (`brew install messense/macos-cross-toolchains/x86_64-unknown-linux-gnu`); on Linux it
# builds natively. The binary is linked against glibc 2.28 or newer.
set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f .deploy-server.env ] && . ./.deploy-server.env
HOST="${TERMLAB_DEPLOY_HOST:?Set TERMLAB_DEPLOY_HOST=user@host, e.g. in .deploy-server.env}"
PORT="${TERMLAB_DEPLOY_PORT:-22}"
SSH=(ssh -p "$PORT" -o BatchMode=yes "$HOST")
BIN=/usr/local/bin/termlab-server

if [ "${1:-}" = "--rollback" ]; then
  "${SSH[@]}" "sudo test -f $BIN.prev && sudo cp $BIN.prev $BIN && sudo systemctl restart termlab-server && $BIN --version"
  echo "✓ Rolled back"
  exit 0
fi

TARGET=x86_64-unknown-linux-gnu
if [ "$(uname -s)" = "Darwin" ]; then
  export CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
fi

echo "→ Building the web UI"
(cd apps/server-web && { [ -d node_modules ] || npm ci; } && npm run build >/dev/null)

echo "→ Building termlab-server for $TARGET"
cargo build --release --target "$TARGET" -p termlab-server
OUT="target/$TARGET/release/termlab-server"

REV="$(git rev-parse --short HEAD)$(git diff --quiet HEAD -- crates/termlab-server apps/server-web || echo '+changes')"
echo "→ Uploading ($REV)"
scp -q -P "$PORT" -o BatchMode=yes "$OUT" "$HOST:/tmp/termlab-server.new"

echo "→ Installing and restarting"
"${SSH[@]}" "set -e
  sudo cp $BIN $BIN.prev
  sudo install -m 0755 -o root -g root /tmp/termlab-server.new $BIN
  rm /tmp/termlab-server.new
  sudo systemctl restart termlab-server
  for i in 1 2 3 4 5 6 7 8 9 10; do
    curl -fs -o /dev/null http://127.0.0.1:8484/api/state && break
    sleep 0.5
  done
  systemctl is-active --quiet termlab-server || { sudo journalctl -u termlab-server -n 20 --no-pager; exit 1; }
  echo \"  \$($BIN --version), running\""

echo "✓ Deployed $REV. Paired computers reconnect by themselves; reload the browser."
