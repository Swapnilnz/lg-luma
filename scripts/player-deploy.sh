#!/bin/sh
# Loop 3: test, build the release bundle, package Luma Player and install it on the TV.
# Run inside `nix develop .#player` from the repo root; for a TV before webOS 24 (Chromium
# < 96), use `nix develop .#player-mvp` instead, which builds plain MVP WebAssembly.
# TV=<ip> overrides the address.
set -eu
TV="${TV:-192.168.6.184}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Native tests: never with build-std (the MVP shell sets it for the wasm build).
env -u CARGO_UNSTABLE_BUILD_STD cargo test --locked -p luma-core
(cd player && cargo clippy --target wasm32-unknown-unknown -- -D warnings)
# Relative paths: the packaged app loads from file://.
(cd player && trunk build --release --public-url ./)

rm -rf out/player && mkdir -p out/player
pnpm exec ares-package --no-minify player/dist -o out/player
IPK="$(ls out/player/com.swapnil.lumaplayer_*.ipk)"

scp -O "$IPK" "root@$TV:/tmp/lumaplayer.ipk"
ssh "root@$TV" 'luna-send -i luna://com.webos.appInstallService/dev/install '\''{"id":"com.swapnil.lumaplayer","ipkUrl":"/tmp/lumaplayer.ipk","subscribe":true}'\'' & sleep 15; kill $! 2>/dev/null'
echo "Installed $(basename "$IPK") on $TV"
