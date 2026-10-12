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

# Projects that run `NumyAddon/ToCVersions` before the packager get their
# `@toc-version-<game>@` placeholders replaced first, with the interface
# numbers of wowlua's flavor table (what `wowlua init` uses).
if grep -rqs "NumyAddon/ToCVersions" "$WORK/addon/.github/workflows"; then
  FLAVORS="$ROOT/crates/wowlua-flavors/data/flavors.json"
  n() { jq -r --arg id "$1" '.flavors[] | select(.id == $id) | .interface[0]' "$FLAVORS"; }
  find "$WORK/addon" -name '*.toc' -not -path '*/.git/*' -exec sed -i \
    -e "s/@toc-version-midnight@/$(n standard)/g" -e "s/@toc-version-retail@/$(n standard)/g" \
    -e "s/@toc-version-camelot@/$(n camelot)/g" -e "s/@toc-version-forever@/$(n camelot)/g" \
    -e "s/@toc-version-mists@/$(n mists)/g" -e "s/@toc-version-cata@/$(n cata)/g" \
    -e "s/@toc-version-wrath@/$(n wrath)/g" -e "s/@toc-version-bcc@/$(n tbc)/g" \
    -e "s/@toc-version-classic@/$(n vanilla)/g" {} +
fi

# wowlua.
(cd "$WORK/wowlua" && "$WOWLUA" init --from-pkgmeta)

# Projects whose CI packages one game per packager run (`-g <game> -m
# <file>`) are compared game by game; the others in one multi-version run.
# The packager: no upload (-d), no zip (-z), no localization (-l, which needs
# the CurseForge API), Unix line endings (-u).
# Projects released with `-S` get TOCs per game from `## Interface-<Game>:`.
RUNS=$(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p xtask -- packager-runs "$WORK/addon")
if [ -z "$RUNS" ] || [ "$RUNS" = split ]; then
  split=()
  [ "$RUNS" = split ] && split=(-S)
  (cd "$WORK/addon" && bash "$WORK/packager/release.sh" -d -z -l -u "${split[@]}" -r "$WORK/release")
  (cd "$WORK/wowlua" && "$WOWLUA" build --release)
  # Every AddOn folder the packager made (move-folders makes several).
  cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p xtask -- packager-diff \
    "$WORK/release" "$WORK/wowlua"
else
  status=0
  while read -r game flavor file; do
    echo "### $game ($flavor) with ${file/#-/the default project file}"
    rm -rf "$WORK/release"
    meta=()
    [ "$file" != "-" ] && meta=(-m "$file")
    (cd "$WORK/addon" && bash "$WORK/packager/release.sh" -d -z -l -u -g "$game" "${meta[@]}" -r "$WORK/release")
    (cd "$WORK/wowlua" && "$WOWLUA" build --release --flavor "$flavor")
    cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p xtask -- packager-diff \
      --flavor "$flavor" "$WORK/release" "$WORK/wowlua" || status=1
  done <<< "$RUNS"
  exit $status
fi
