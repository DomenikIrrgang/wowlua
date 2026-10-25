#!/bin/sh
# Runs the extension's tests in a VS Code of its own (a fresh profile, no
# other extensions) on examples/widget-demo, with target/release/wowlua.
# Needs VS Code, a WoW installation and `cargo build --release -p wowlua-cli`
# (or WOWLUA_BIN); WOWLUA_SCENARIO=lsp, complete or chat runs that scenario
# alone. WOWLUA_FOLDER opens another folder (with WOWLUA_COMPLETE_FILE, the
# file the complete scenario types in; WOWLUA_SETTLE, how long it waits for
# the first check, in ms), WOWLUA_USER_SETTINGS gives the profile a
# settings.json, WOWLUA_EXTENSIONS loads the extensions of that folder.
set -e
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
code=${VSCODE:-"/Applications/Visual Studio Code.app/Contents/MacOS/Code"}
work=$(mktemp -d)
log="$work/run.log"
: > "$log"
# WOWLUA_USER_SETTINGS: a settings.json for the test's profile
if [ -n "$WOWLUA_USER_SETTINGS" ]; then mkdir -p "$work/user/User" && cp "$WOWLUA_USER_SETTINGS" "$work/user/User/settings.json"; fi
status=0
WOWLUA_VSCODE_LOG="$log" WOWLUA_BIN="${WOWLUA_BIN:-$repo/target/release/wowlua}" "$code" \
    --user-data-dir="$work/user" --extensions-dir="${WOWLUA_EXTENSIONS:-$work/extensions}" \
    ${WOWLUA_EXTENSIONS:+--enable-proposed-api} ${WOWLUA_EXTENSIONS:---disable-extensions} --disable-workspace-trust --skip-welcome --skip-release-notes \
    --extensionDevelopmentPath="$repo/editors/vscode" \
    --extensionTestsPath="$here/suite.js" \
    "${WOWLUA_FOLDER:-$repo/examples/widget-demo}" > "$work/vscode.out" 2>&1 || status=$?
grep -E "^==|PASS|FAIL|Error|ALL|typed" "$log" || true
if [ "$status" -eq 0 ]; then rm -rf "$work"; else echo "logs: $work"; fi
exit $status
