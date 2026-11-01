#!/usr/bin/env bash
# The WSL side of build-linux.bat; runs in the project folder on the Windows drive.
# Builds in a copy on the Linux file system: the Windows folder has node_modules for Windows,
# and building on /mnt/c is slow. The packages are copied back into release/<version>/.
set -euo pipefail
src="$PWD"
work=/root/audio-graph

version=$(node -p "require('./package.json').version" 2>/dev/null || grep -m1 '"version"' package.json | cut -d'"' -f4)
if [ -e "$src/release/$version/Audio Graph $version.AppImage" ]; then
    echo "Version $version is already built: release/$version/Audio Graph $version.AppImage" >&2
    echo "Change the version first: npm version patch" >&2
    exit 1
fi

command -v rsync >/dev/null || { apt-get update -q && apt-get install -y -q rsync; }
mkdir -p "$work"
rsync -a --delete \
    --exclude node_modules --exclude dist --exclude release --exclude portable --exclude .git \
    --exclude src-tauri/target --exclude src-tauri/gen \
    "$src/" "$work/"
cd "$work"
rm -rf release

# build tools: installed on the first run and again whenever setup.sh changes
marker=/root/.audio-graph-setup
if ! cmp -s scripts/linux/setup.sh "$marker"; then
    bash scripts/linux/setup.sh
    cp scripts/linux/setup.sh "$marker"
fi

bash scripts/linux/build.sh
mkdir -p "$src/release/$version"
cp "release/$version/"* "$src/release/$version/"
