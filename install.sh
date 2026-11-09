#!/usr/bin/env sh
# Builds wowmod in release mode and installs it onto your PATH.
#
# Runs `cargo install --path <repo> --locked`, which puts the binary in
# Cargo's bin directory ($CARGO_HOME/bin, default ~/.cargo/bin). rustup adds
# that directory to PATH, so afterwards `wowmod` works from any shell.
#
# Re-run after pulling changes to upgrade. Remove with: cargo uninstall wowmod
set -eu

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo was not found. Install Rust from https://rustup.rs, open a new shell, and re-run this script." >&2
    exit 1
fi

repo=$(cd "$(dirname "$0")" && pwd)
echo "Installing wowmod from $repo ..."
cargo install --path "$repo" --locked

bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
case ":$PATH:" in
    *":$bin_dir:"*)
        echo
        echo "Installed: $bin_dir/wowmod"
        echo "Run 'wowmod' from any shell. Re-run this script to upgrade, 'cargo uninstall wowmod' to remove."
        ;;
    *)
        echo
        echo "warning: $bin_dir is not on your PATH. Add this to your shell profile and open a new shell:" >&2
        echo "    export PATH=\"$bin_dir:\$PATH\"" >&2
        ;;
esac
