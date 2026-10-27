#!/bin/sh
# Loop 2, once: installs com.swapnil.lumaplayer.dev, a tiny app that opens the Trunk server
# on this laptop with the launch params, so every save reloads the player on the TV.
#   scripts/player-dev-shell.sh 192.168.6.10     (this laptop's address on the TV's network)
set -eu
TV="${TV:-192.168.6.184}"
LAPTOP="${1:?usage: scripts/player-dev-shell.sh LAPTOP_IP}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$WORK/app" "$WORK/out"
cp "$ROOT"/player/dev-shell/* "$WORK/app/"
sed "s/LAPTOP_IP/$LAPTOP/g" "$ROOT/player/dev-shell/index.html" > "$WORK/app/index.html"
(cd "$ROOT" && pnpm exec ares-package --no-minify "$WORK/app" -o "$WORK/out")
IPK="$(ls "$WORK"/out/com.swapnil.lumaplayer.dev_*.ipk)"

scp -O "$IPK" "root@$TV:/tmp/lumaplayer-dev.ipk"
ssh "root@$TV" 'luna-send -i luna://com.webos.appInstallService/dev/install '\''{"id":"com.swapnil.lumaplayer.dev","ipkUrl":"/tmp/lumaplayer-dev.ipk","subscribe":true}'\'' & sleep 15; kill $! 2>/dev/null'
echo "Dev shell installed: it opens http://$LAPTOP:8080"
