#!/usr/bin/env bash
# Runs the iPhone app in a simulator, in its demo (a desktop inside the app: phone/src-tauri's
# demo.rs), and saves a screenshot of each screen. This is how the app gets looked at without a
# Mac or an iPhone at hand.
#
#   ios-simulator.sh <path/to/oah.app> <output dir>
set -euo pipefail
app=$1
out=$2
bundle=dev.oah.mobile
mkdir -p "$out"

# An iPhone Pro on the runtime that matches the SDK the app was built with (else the newest).
sdk=$(xcrun --sdk iphonesimulator --show-sdk-version | cut -d. -f1)
udid=$(xcrun simctl list devices available -j | SDK="$sdk" python3 -c '
import json, os, sys
best = None
for runtime, devices in json.load(sys.stdin)["devices"].items():
    if "iOS-" not in runtime:
        continue
    version = tuple(int(x) for x in runtime.rsplit("iOS-", 1)[-1].split("-"))
    for d in devices:
        name = d["name"]
        if not name.startswith("iPhone"):
            continue
        key = (str(version[0]) == os.environ["SDK"], "Pro" in name and "Max" not in name, version, name)
        if best is None or key > best[0]:
            best = (key, d["udid"], name, runtime)
if best is None:
    sys.exit("no iPhone simulator is available")
print(best[2], "on", best[3], file=sys.stderr)
print(best[1])
')
# The on-screen keyboard, not a Mac's (for the UI test that looks at it).
defaults write com.apple.iphonesimulator ConnectHardwareKeyboard -bool false || true
echo "$udid" > "$out/udid"
xcrun simctl boot "$udid" || true
xcrun simctl bootstatus "$udid" -b
xcrun simctl status_bar "$udid" override --time "9:41" --batteryState charged --batteryLevel 100 --cellularBars 4 --wifiBars 3 || true
xcrun simctl install "$udid" "$app"

# shot <name> <route|-> [demo: 1|0]: starts the app on a screen and saves what it shows.
shot() {
  local name=$1 route=$2 demo=${3:-1}
  xcrun simctl terminate "$udid" "$bundle" >/dev/null 2>&1 || true
  if [ "$route" = "-" ]; then
    SIMCTL_CHILD_OAH_DEMO="$demo" xcrun simctl launch "$udid" "$bundle"
  else
    SIMCTL_CHILD_OAH_DEMO="$demo" SIMCTL_CHILD_OAH_ROUTE="$route" xcrun simctl launch "$udid" "$bundle"
  fi
  sleep 8
  xcrun simctl io "$udid" screenshot "$out/$name.png"
}

xcrun simctl ui "$udid" appearance light || true
shot 01-first-launch - 0
shot 02-chats /
shot 03-chat-approval /chat/c-checkout
shot 04-chat-question /chat/c-dates
shot 05-chat-finished /chat/c-invoices
shot 06-chat-diff /chat/c-rename
shot 07-pair /pair
shot 08-settings /settings
shot 09-desktops /desktops
shot 10-new-chat /new/p-store
xcrun simctl ui "$udid" appearance dark || true
shot 11-dark-chats /
shot 12-dark-chat-approval /chat/c-checkout
shot 13-dark-chat-finished /chat/c-invoices
# The page's own sizes, on the screen (there's no console to read here).
xcrun simctl ui "$udid" appearance light || true
xcrun simctl terminate "$udid" "$bundle" >/dev/null 2>&1 || true
SIMCTL_CHILD_OAH_DEMO=1 SIMCTL_CHILD_OAH_DEBUG=1 SIMCTL_CHILD_OAH_ROUTE=/chat/c-invoices xcrun simctl launch "$udid" "$bundle"
sleep 8
xcrun simctl io "$udid" screenshot "$out/14-sizes.png"
# Ten seconds in, the app lays its web view out afresh and shows the sizes again.
sleep 7
xcrun simctl io "$udid" screenshot "$out/15-sizes-after-layout.png"

xcrun simctl terminate "$udid" "$bundle" >/dev/null 2>&1 || true
# What the app said, and any crash.
xcrun simctl spawn "$udid" log show --last 10m --style compact --predicate 'process == "oah"' 2>/dev/null | tail -n 400 > "$out/app.log" || true
cp ~/Library/Logs/DiagnosticReports/oah* "$out"/ 2>/dev/null || true
ls -la "$out"
