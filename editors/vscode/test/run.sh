#!/bin/sh
# Runs the extension's tests in a VS Code of its own (a fresh profile, no
# other extensions) on examples/widget-demo, with target/release/wowlua.
# Needs VS Code, a WoW installation and `cargo build --release -p wowlua-cli`
# (or WOWLUA_BIN); WOWLUA_SCENARIO=lsp or chat runs that scenario alone.
set -e
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
code=${VSCODE:-"/Applications/Visual Studio Code.app/Contents/MacOS/Code"}
work=$(mktemp -d)
log="$work/run.log"
: > "$log"
status=0
WOWLUA_VSCODE_LOG="$log" WOWLUA_BIN="${WOWLUA_BIN:-$repo/target/release/wowlua}" "$code" \
    --user-data-dir="$work/user" --extensions-dir="$work/extensions" \
    --disable-extensions --disable-workspace-trust --skip-welcome --skip-release-notes \
    --extensionDevelopmentPath="$repo/editors/vscode" \
    --extensionTestsPath="$here/suite.js" \
    "$repo/examples/widget-demo" > "$work/vscode.out" 2>&1 || status=$?
grep -E "^==|PASS|FAIL|Error|ALL" "$log" || true
if [ "$status" -eq 0 ]; then rm -rf "$work"; else echo "logs: $work"; fi
exit $status
