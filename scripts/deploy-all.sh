#!/bin/sh
# Builds and installs everything on the TV: Luma Player, feeds.json, and Luma (as Home).
# Run from anywhere in the repo, in a plain shell (it enters the Nix shells itself):
#
#   sh scripts/deploy-all.sh                 # everything
#   sh scripts/deploy-all.sh --player        # only Luma Player
#   sh scripts/deploy-all.sh --luma          # only Luma (and feeds.json)
#   sh scripts/deploy-all.sh --no-feeds      # leave feeds.json on the TV as it is
#   TV=192.168.6.50 sh scripts/deploy-all.sh # another TV
set -eu

TV="${TV:-192.168.6.184}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DO_PLAYER=1
DO_LUMA=1
DO_FEEDS=1
for arg in "$@"; do
  case "$arg" in
    --player) DO_LUMA=0 ;;
    --luma) DO_PLAYER=0 ;;
    --no-feeds) DO_FEEDS=0 ;;
    -h | --help)
      sed -n '2,9p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown option: $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

nixdev() {
  nix --extra-experimental-features "nix-command flakes" develop "$@"
}
step() {
  printf '\n\033[1m== %s\033[0m\n' "$*"
}
fail() {
  printf '\033[31m%s\033[0m\n' "$*" >&2
  exit 1
}
tv() {
  ssh -o ConnectTimeout=8 "root@$TV" "$@"
}
# Installs an IPK already copied to /tmp on the TV. The install service only answers on a
# subscription, so it is stopped after a while rather than waited on; its last replies show.
install_ipk() {
  tv "luna-send -i luna://com.webos.appInstallService/dev/install '{\"id\":\"$1\",\"ipkUrl\":\"$2\",\"subscribe\":true}' > /tmp/install.log 2>&1 & sleep $3; kill \$! 2>/dev/null; tail -3 /tmp/install.log"
}

step "Checks"
# Windows line endings break these scripts on the TV and in sh.
if grep -lq "$(printf '\r')" scripts/*.sh tv/*.sh 2>/dev/null; then
  fail "Some scripts have Windows (CRLF) line endings. Run: sed -i 's/\r\$//' scripts/*.sh tv/*.sh"
fi
command -v nix > /dev/null || fail "nix is not installed"
tv true || fail "Can't reach root@$TV over SSH (is the TV on, and the address right?)"
echo "TV: $TV ($(tv 'df -h /media/developer | tail -1' | awk '{print $4 " free"}'))"
if [ "$DO_PLAYER" = 1 ] && [ ! -f player/config.local.json ]; then
  fail "player/config.local.json is missing: cp player/config.example.json player/config.local.json and fill it in"
fi
if [ "$DO_LUMA" = 1 ] && [ "$DO_FEEDS" = 1 ]; then
  [ -f feeds.json ] || fail "feeds.json is missing (or pass --no-feeds)"
  grep -q '"player"' feeds.json || echo "Note: feeds.json has no \"player\" block, so posters still open Nuvio"
fi
if [ ! -d node_modules ]; then
  step "pnpm install (first run)"
  nixdev -c pnpm install --frozen-lockfile
fi

if [ "$DO_PLAYER" = 1 ]; then
  step "Luma Player: test, build, package, install"
  nixdev .#player -c sh scripts/player-deploy.sh
fi

if [ "$DO_LUMA" = 1 ]; then
  if [ "$DO_FEEDS" = 1 ]; then
    step "feeds.json to the folder Home reads"
    scp -O feeds.json "root@$TV:/media/developer/temp/com.ecklf.luma/feeds.json"
  fi

  if [ ! -d .build/toolchain/arm-webos-linux-gnueabi_sdk-buildroot ]; then
    step "webOS toolchain (first run)"
    nixdev -c pnpm sdk
  fi

  step "Luma: build and package"
  nixdev -c sh -c 'export WEBOS_NDK="$PWD/.build/toolchain/arm-webos-linux-gnueabi_sdk-buildroot" && pnpm assets && pnpm run package'
  version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' package.json | head -1)"
  ipk="dist/com.ecklf.luma_${version}_arm.ipk"
  [ -f "$ipk" ] || fail "Expected $ipk after packaging"

  step "Luma: install and take over Home"
  scp -O "$ipk" "root@$TV:/tmp/luma.ipk"
  install_ipk com.ecklf.luma /tmp/luma.ipk 25
  tv 'sh /media/developer/apps/usr/palm/applications/com.ecklf.luma/home.sh enable'
fi

step "Tidy up"
tv 'rm -f /tmp/luma.ipk /tmp/lumaplayer.ipk'
echo "Done. Luma's log: ssh root@$TV 'tail -f /tmp/luma-home.log'"
