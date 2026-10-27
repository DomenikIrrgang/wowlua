#!/bin/sh
# Installs the latest (or a given) wowlua release on macOS and Linux.
#   curl -fsSL https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.sh | sh
#   WOWLUA_VERSION=v0.1.0 WOWLUA_INSTALL_DIR=$HOME/bin sh install.sh
set -eu

repo="DomenikIrrgang/wowlua"
version="${WOWLUA_VERSION:-latest}"
install_dir="${WOWLUA_INSTALL_DIR:-$HOME/.wowlua/bin}"

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux) os="unknown-linux-gnu" ;;
  *) echo "unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
target="$arch-$os"

if [ "$version" = "latest" ]; then
  # The release page redirects to the latest tag; unlike GitHub's API it has
  # no rate limit for requests without a token.
  latest="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")" || latest=""
  version="${latest##*/tag/}"
  case "$version" in
    v*) ;;
    *) echo "could not find the latest wowlua release; set WOWLUA_VERSION (for example v0.13.0)" >&2; exit 1 ;;
  esac
fi
name="wowlua-$version-$target"
url="https://github.com/$repo/releases/download/$version/$name.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "downloading $url"
curl -fsSL "$url" -o "$tmp/$name.tar.gz"
curl -fsSL "$url.sha256" -o "$tmp/$name.tar.gz.sha256"
(cd "$tmp" && shasum -a 256 -c "$name.tar.gz.sha256" >/dev/null)
tar xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$install_dir"
install -m 755 "$tmp/$name/wowlua" "$install_dir/wowlua"
echo "installed wowlua $version to $install_dir/wowlua"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "add $install_dir to your PATH" ;;
esac
