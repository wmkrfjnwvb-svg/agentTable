#!/usr/bin/env bash
# Prints the package's path relative to the repository root.
# Assembles a release package: the server binary (with the web app embedded), start scripts
# and instructions, as a .zip (Windows) or .tar.gz.
#   scripts/package.sh <binary> <platform-name>     e.g. target/release/agenttable linux-x86_64
set -euo pipefail
binary="$1"
platform="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
name="agenttable-$version-$platform"
out="$root/dist/$name"
rm -rf "$out" && mkdir -p "$out"
cp "$root/scripts/package/README.txt" "$out/"
case "$platform" in
  windows-*)
    cp "$binary" "$out/agenttable.exe"
    cp "$root/scripts/package/start.bat" "$out/"
    (cd "$root/dist" && 7z a -tzip "$name.zip" "$name" >/dev/null)
    echo "dist/$name.zip"
    ;;
  *)
    cp "$binary" "$out/agenttable"
    chmod +x "$out/agenttable"
    if [[ "$platform" == macos-* ]]; then
      cp "$root/scripts/package/start.command" "$out/" && chmod +x "$out/start.command"
    else
      cp "$root/scripts/package/start.sh" "$out/" && chmod +x "$out/start.sh"
    fi
    tar -C "$root/dist" -czf "$root/dist/$name.tar.gz" "$name"
    echo "dist/$name.tar.gz"
    ;;
esac
