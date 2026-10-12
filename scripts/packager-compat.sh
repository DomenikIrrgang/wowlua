#!/usr/bin/env bash
# The M6 exit check (spec roadmap): an Ace3-based addon adopted with
# `wowlua init --from-pkgmeta` builds the same code as the BigWigs packager.
#
# Both tools package the same commit at the same time (so unpinned externals
# resolve to the same revisions); `cargo xtask packager-diff` then compares
# the loaded files, assets and TOC directives per flavor.
#
# Needs bash 4+, git, svn, curl and unzip (the packager's requirements).
set -euo pipefail

ADDON_REPO=${ADDON_REPO:-https://github.com/Nevcairiel/Bartender4}
ADDON_REV=${ADDON_REV:-8339c74c5ddb5cd93bf789235774bcce04123155}
PACKAGER_REV=${PACKAGER_REV:-0f24d2169de3ceb3b79a981311b2ebab4c6160c7}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=${WORK:-$ROOT/target/packager-compat}
WOWLUA=${WOWLUA:-$ROOT/target/release/wowlua}
# Outside target/, which CI caches between runs.
export WOWLUA_HOME=${WOWLUA_HOME:-$(mktemp -d)}

rm -rf "$WORK/addon" "$WORK/wowlua" "$WORK/release" "$WORK/packager"
mkdir -p "$WORK"
git clone -q "$ADDON_REPO" "$WORK/addon"
git -C "$WORK/addon" checkout -q "$ADDON_REV"
git clone -q https://github.com/BigWigsMods/packager "$WORK/packager"
git -C "$WORK/packager" checkout -q "$PACKAGER_REV"
cp -R "$WORK/addon" "$WORK/wowlua"

# The packager: no upload (-d), no zip (-z), no localization (-l, which needs
# the CurseForge API), Unix line endings (-u; wowlua passes files through
# unchanged).
(cd "$WORK/addon" && bash "$WORK/packager/release.sh" -d -z -l -u -r "$WORK/release")

# wowlua.
(cd "$WORK/wowlua" && "$WOWLUA" init --from-pkgmeta && "$WOWLUA" build --release)

# Every AddOn folder the packager made (move-folders makes several).
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p xtask -- packager-diff \
  "$WORK/release" "$WORK/wowlua"
