#!/usr/bin/env bash
# Builds Audio Graph for Linux into release/<version>/:
#   Audio Graph <version>.AppImage     - a single file, no installation
#   audio-graph_<version>_amd64.deb    - Debian, Ubuntu, Mint
#   audio-graph-<version>.x86_64.rpm   - Fedora, openSUSE
# The version comes from package.json; change it with "npm version patch" (or minor, major).
# The packages run on distributions as old as the one they are built on or newer (glibc):
# for releases build on Ubuntu 22.04, which scripts/windows/build-linux.bat does from Windows.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

version=$(node -p "require('./package.json').version")
out="release/$version"
appimage="$out/Audio Graph $version.AppImage"

# one version, one build: a changed app gets a new version number
if [ -e "$appimage" ]; then
    echo "Version $version is already built: $appimage" >&2
    echo "Change the version first: npm version patch" >&2
    exit 1
fi

npm install --no-audit --no-fund

# linuxdeploy (the AppImage builder) is an AppImage itself: run it without FUSE, which
# containers and WSL may lack
export APPIMAGE_EXTRACT_AND_RUN=1
npm run tauri build -- --bundles appimage,deb,rpm

arch=$(uname -m)
deb_arch=$([ "$arch" = x86_64 ] && echo amd64 || echo "$arch")
bundle=src-tauri/target/release/bundle
mkdir -p "$out"
cp "$bundle/deb/Audio Graph_${version}_${deb_arch}.deb" "$out/audio-graph_${version}_${deb_arch}.deb"
cp "$bundle/rpm/Audio Graph-${version}-1.${arch}.rpm" "$out/audio-graph-${version}.${arch}.rpm"
cp "$bundle/appimage/Audio Graph_${version}_${deb_arch}.AppImage" "$appimage"

echo
echo "Done:"
ls -1 "$out" | grep -v '\.exe$\|\.msi$' | sed "s|^|  $out/|"
