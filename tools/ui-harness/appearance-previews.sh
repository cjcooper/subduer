#!/bin/sh
# Regenerates ui/assets/appearance-{light,dark}.jpg, the Settings › Appearance
# previews: the workloadstub Deployment (generic acme data, mid-rollout) at
# 1440x900, shrunk to 560px wide. Run from the repo root after UI changes:
#   sh tools/ui-harness/appearance-previews.sh
set -eu
out=$(mktemp -d)
trunk build --dist "$out/build" >/dev/null
cp tools/ui-harness/*.js "$out/build/"
sed 's|<head>|<head><script src="workloadstub.js"></script>|' "$out/build/index.html" > "$out/build/w.html"
(cd "$out/build" && python3 -m http.server 8790 >/dev/null 2>&1) &
server=$!
trap 'kill $server' EXIT
sleep 1
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
for pair in light:1 dark:0; do
  name=${pair%%:*}
  scheme=${pair##*:}
  "$chrome" --headless=new --disable-gpu --hide-scrollbars --blink-settings=preferredColorScheme=$scheme \
    --force-device-scale-factor=1 --window-size=1440,900 --virtual-time-budget=7000 \
    --screenshot="$out/$name.png" "http://localhost:8790/w.html?rolling=1" 2>/dev/null
  sips -s format jpeg -s formatOptions 82 -Z 560 "$out/$name.png" --out "ui/assets/appearance-$name.jpg" >/dev/null
done
echo "updated ui/assets/appearance-light.jpg and appearance-dark.jpg"
