#!/usr/bin/env sh
# Installs the wowmod command-line tool to ~/.local/bin (macOS / Linux).
#
#   Default:  downloads the latest GitHub release (or a tag: ./install.sh v0.1.0)
#   --local:  installs target/release/wowmod you already built with
#             `cargo build --release`, overwriting any installed release build.
#
#   Users:       curl -fsSL https://raw.githubusercontent.com/zachbrown-bm/wowmod/main/install.sh | sh
#   Developers:  cargo build --release && ./install.sh --local
#
# Set WOWMOD_INSTALL_DIR to install somewhere other than ~/.local/bin.
set -eu

repo="zachbrown-bm/wowmod"
install_dir="${WOWMOD_INSTALL_DIR:-$HOME/.local/bin}"
target="$install_dir/wowmod"
mode=release
version=latest

for arg in "$@"; do
    case "$arg" in
        --local) mode=local ;;
        -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) version="$arg" ;;
    esac
done

mkdir -p "$install_dir"

if [ "$mode" = local ]; then
    here=$(cd "$(dirname "$0")" && pwd)
    built="$here/target/release/wowmod"
    if [ ! -f "$built" ]; then
        echo "No release build at $built. Run 'cargo build --release' first." >&2
        exit 1
    fi
    echo "Installing local build $built"
    cp "$built" "$target"
else
    os=$(uname -s); arch=$(uname -m)
    case "$os" in
        Darwin) os=macos ;;
        Linux)  os=linux ;;
        *) echo "Unsupported OS: $os. Use install.ps1 on Windows." >&2; exit 1 ;;
    esac
    case "$arch" in
        x86_64|amd64)  arch=x86_64 ;;
        arm64|aarch64) arch=aarch64 ;;
        *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
    esac
    asset="wowmod-$os-$arch.tar.gz"
    if [ "$version" = latest ]; then
        url="https://github.com/$repo/releases/latest/download/$asset"
    else
        url="https://github.com/$repo/releases/download/$version/$asset"
    fi

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "Downloading $url"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$tmp/$asset"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$tmp/$asset"
    else
        echo "Need curl or wget to download the release." >&2; exit 1
    fi
    tar -xzf "$tmp/$asset" -C "$tmp"
    cp "$tmp/wowmod" "$target"
fi
chmod +x "$target"

echo
echo "Installed: $target"
"$target" --version

case ":$PATH:" in
    *":$install_dir:"*) ;;
    *)
        echo
        echo "warning: $install_dir is not on your PATH. Add this to your shell profile and open a new shell:" >&2
        echo "    export PATH=\"$install_dir:\$PATH\"" >&2
        ;;
esac

other=$(command -v wowmod 2>/dev/null || true)
if [ -n "$other" ] && [ "$other" != "$target" ]; then
    echo
    echo "warning: '$other' is earlier on your PATH and will run instead of $target." >&2
    echo "Remove it (for a cargo install: 'cargo uninstall wowmod') or put $install_dir first." >&2
fi
