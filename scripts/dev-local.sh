#!/bin/sh
# Runs everything on this computer: Luma's desktop preview, with Luma Player served live by
# Trunk. OK on a poster opens the title in Luma Player in your browser; saving a player file
# rebuilds it and reloads the page. Run from anywhere in the repo, in a plain shell:
#
#   sh scripts/dev-local.sh            # Luma preview + Luma Player
#   sh scripts/dev-local.sh --player   # Luma Player only, opened on a sample episode
#   PORT=8090 NIXGL=nixGLNvidia sh scripts/dev-local.sh
#
# Closing Luma's window (or Ctrl+C) stops the player's server too.
set -eu


# --- GL launcher: nixGL is Linux-only, so choose per platform --------------
# GL_RUN is a command prefix (empty on macOS). Exported so nested sh -c sees it.
case "$(uname -s)" in
  Darwin)
    GL_RUN="" ;;
  Linux)
    GL_RUN="nix --extra-experimental-features nix-command --extra-experimental-features flakes run --impure github:nix-community/nixGL#${NIXGL:-nixGLIntel} --"
    if grep -qi microsoft /proc/version 2>/dev/null; then
      if [ "${LUMA_GL:-}" = software ]; then
        GL_RUN="$GL_RUN env LIBGL_ALWAYS_SOFTWARE=1"
      else
        GL_RUN="$GL_RUN env LD_LIBRARY_PATH=/usr/lib/wsl/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH} GALLIUM_DRIVER=${GALLIUM_DRIVER:-d3d12}"
      fi
    fi ;;
  *)
    echo "dev-local.sh: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
export GL_RUN
# ---------------------------------------------------------------------------

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PORT="${PORT:-8080}"
NIXGL="${NIXGL:-nixGLIntel}"
PLAYER_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --player) PLAYER_ONLY=1 ;;
    -h | --help)
      sed -n '2,11p' "$0"
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
note() {
  printf '\033[33m%s\033[0m\n' "$*"
}
fail() {
  printf '\033[31m%s\033[0m\n' "$*" >&2
  exit 1
}
# Opens a URL in the browser: Windows' under WSL, else the desktop's.
browse() {
  if grep -qi microsoft /proc/version 2>/dev/null; then
    command -v wslview > /dev/null && wslview "$1" || explorer.exe "$1" || true
  elif command -v xdg-open > /dev/null; then
    xdg-open "$1" > /dev/null 2>&1 &
  elif command -v open > /dev/null; then
    open "$1"
  else
    echo "Open $1 in your browser"
  fi
}

step "Checks"
command -v nix > /dev/null || fail "nix is not installed"
command -v curl > /dev/null || fail "curl is needed to wait for the player's server"
if [ ! -f player/config.local.json ]; then
  note "player/config.local.json is missing: the player builds with the example config, so"
  note "streams, Trakt and TMDB won't work (cp player/config.example.json player/config.local.json)"
fi
if [ "$PLAYER_ONLY" = 0 ]; then
  if [ ! -f feeds.json ]; then
    note "No feeds.json in the repo folder: the preview shows no rows (see feeds.example.json)"
  elif ! grep -q '"player"' feeds.json; then
    note "feeds.json has no \"player\" block: posters won't open Luma Player"
  fi
fi
if [ ! -d node_modules ]; then
  step "pnpm install (first run)"
  nixdev -c pnpm install --frozen-lockfile
fi

step "Luma Player: dev server on port $PORT"
mkdir -p .build
LOG=.build/player-dev.log
SERVE="cd player && exec trunk serve --address 0.0.0.0 --port $PORT --features dev"
# In its own process group where possible, so stopping it also stops trunk (a child of nix).
if command -v setsid > /dev/null; then
  setsid nix --extra-experimental-features "nix-command flakes" develop .#player -c sh -c "$SERVE" > "$LOG" 2>&1 &
  GROUP=1
else
  nix --extra-experimental-features "nix-command flakes" develop .#player -c sh -c "$SERVE" > "$LOG" 2>&1 &
  GROUP=0
fi
SERVER=$!
stop_server() {
  if [ "$GROUP" = 1 ]; then
    kill -TERM -- "-$SERVER" 2> /dev/null || true
  fi
  kill "$SERVER" 2> /dev/null || true
  pkill -f "trunk serve --address 0.0.0.0 --port $PORT" 2> /dev/null || true
}
trap stop_server EXIT INT TERM

printf 'Building (the first build takes a few minutes; log: %s) ' "$LOG"
waited=0
until curl -sf -o /dev/null "http://127.0.0.1:$PORT/"; do
  if ! kill -0 "$SERVER" 2> /dev/null; then
    echo
    tail -25 "$LOG"
    fail "The player's server stopped (see above)"
  fi
  if [ "$waited" -ge 600 ]; then
    echo
    tail -25 "$LOG"
    fail "The player's server didn't come up in 10 minutes"
  fi
  printf '.'
  sleep 3
  waited=$((waited + 3))
done
echo " up: http://localhost:$PORT/"

if [ "$PLAYER_ONLY" = 1 ]; then
  # Game of Thrones S1 E2: shows the Episodes and Next Episode buttons.
  browse "http://localhost:$PORT/?params=%7B%22type%22%3A%22series%22%2C%22imdb%22%3A%22tt0944947%22%2C%22tmdb%22%3A1399%2C%22season%22%3A1%2C%22episode%22%3A2%7D"
  echo "Ctrl+C stops the server."
  wait "$SERVER"
  exit 0
fi

step "Luma: desktop preview (OK on a poster opens Luma Player in the browser)"
nixdev -c sh -c "pnpm assets && exec $GL_RUN env LUMA_MSAA=0 LUMA_PLAYER_URL=http://localhost:$PORT/ pnpm dev --preferences \"\$PWD/preferences.json\""
