#!/bin/zsh
# Builds and (re)installs the VibeRooms server as a launchd agent on macOS.
# Every VR_* variable set when running this script is written into the agent, e.g.:
#   VR_HOST=127.0.0.1,100.64.0.10 VR_HOMEPAGE_DIR=~/.config/homepage ./deploy/macos/install-server.sh
set -euo pipefail
export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
ROOT=${0:A:h:h:h}
LABEL=com.allanbruyere.viberooms.server
PLIST=$HOME/Library/LaunchAgents/$LABEL.plist
PORT=${VR_PORT:-8790}

cd "$ROOT"
cargo build --release -p viberooms
mkdir -p "$ROOT/bin"
cp target/release/viberooms "$ROOT/bin/viberooms.new" && mv -f "$ROOT/bin/viberooms.new" "$ROOT/bin/viberooms"

envxml=""
for kv in ${(f)"$(env | grep '^VR_' | sort)"}; do
  val=${kv#*=}; val=${val//&/&amp;}; val=${val//</&lt;}; val=${val//>/&gt;}  # XML escaping
  envxml+="    <key>${kv%%=*}</key><string>${val}</string>"$'\n'
done
cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key><array><string>$ROOT/bin/viberooms</string></array>
  <key>EnvironmentVariables</key>
  <dict>
$envxml  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/viberooms.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/viberooms.log</string>
</dict>
</plist>
PLIST
launchctl bootout gui/$(id -u)/$LABEL 2>/dev/null || true
sleep 1
launchctl bootstrap gui/$(id -u) "$PLIST"
for _ in {1..20}; do
  curl -fs "http://127.0.0.1:$PORT/api/health" && { echo; echo "VibeRooms is running — logs: ~/Library/Logs/viberooms.log"; exit 0; }
  sleep 0.5
done
echo "VibeRooms did not answer, see ~/Library/Logs/viberooms.log" >&2
exit 1
