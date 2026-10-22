#!/usr/bin/env bash
# Build the cute-codex native files cutex runs (codex-app-server, codex, codex-code-mode-host and the
# app-server protocol schema) and pack them as one archive per platform:
#
#   native-<version>-<platform>.tar.gz   codex-app-server[.exe], codex[.exe], codex-code-mode-host[.exe],
#                                        codex_app_server_protocol.schemas.json, cute-codex-build.json,
#                                        LICENSE, NOTICE, THIRD_PARTY_NOTICES.md
#
# This is the archive format of a cutex release manifest's `native.artifacts` (cutex src/upgrade/stage.rs:
# the three files at the archive root, ustar, no directories or links; it unpacks every file and needs only
# those three, so the license files land next to them in natives/<version>/) and of the binary install
# (docs/install/AGENT-INSTALL.md of cutex-desktop).
#
#   bash .github/cute-codex/build-natives.sh <platform> <out-dir>
#
# <platform>: linux-x64 | windows-x64 (Git Bash on Windows, MSVC on PATH as on GitHub's runners).
# Runs in a clean checkout of this repository (core.autocrlf=false: the runtime embeds SQL migrations
# whose checksums must match the committed bytes). Environment (all optional):
#   CARGO_HOME       cargo's home (default ~/.cargo); remapped to /cargo in the binaries
#   V8_DIR           where the prebuilt V8 archive is downloaded (default <repo>/../v8-download)
#   LEAK_POLICY      fail (default) | warn: what a build path found inside a binary does
#   CARGO_TARGET_DIR cargo's target dir (default codex-rs/target); remapped to /target
set -euo pipefail

platform="${1:?platform: linux-x64 | windows-x64}"
out="${2:?out dir}"
case "$platform" in
  linux-x64) target=x86_64-unknown-linux-gnu; exe="" ;;
  windows-x64) target=x86_64-pc-windows-msvc; exe=".exe" ;;
  *) echo "unknown platform $platform" >&2; exit 2 ;;
esac

# Git Bash on Windows: directories given as C:\... become /c/... (GitHub passes RUNNER_TEMP that way).
unixpath() { if command -v cygpath > /dev/null 2>&1; then cygpath -u "$1"; else printf '%s' "$1"; fi; }
out="$(unixpath "$out")"
root="$(cd "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cd "$root"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
mkdir -p "$out"
out="$(cd "$out" && pwd -P)"
v8_dir="$(unixpath "${V8_DIR:-$root/../v8-download}")"
mkdir -p "$v8_dir"
v8_dir="$(cd "$v8_dir" && pwd -P)"

winpath() { if [ "$platform" = windows-x64 ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
mixpath() { if [ "$platform" = windows-x64 ]; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
sha256() { sha256sum "$1" | cut -d' ' -f1; }

# 1. The sources are the committed bytes: no local changes, no file checked out with CRLF that is LF in git.
commit="$(git rev-parse HEAD)"
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "the checkout has local changes; build from a clean checkout" >&2
  git status --porcelain --untracked-files=no | head -20 >&2
  exit 1
fi
crlf="$(git ls-files --eol | awk '$1 == "i/lf" && $2 == "w/crlf"' | head -5)"
if [ -n "$crlf" ]; then
  echo "files are checked out with CRLF (set core.autocrlf=false before the checkout):" >&2
  printf '%s\n' "$crlf" >&2
  exit 1
fi

# 2. The prebuilt V8 that codex-code-mode-host links, verified against the checksums pinned in the tree
#    (same files and checks as .github/actions/setup-rusty-v8).
v8="$(awk '/^name = "v8"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' codex-rs/Cargo.lock)"
[ -n "$v8" ] || { echo "no v8 crate in codex-rs/Cargo.lock" >&2; exit 1; }
profile=ptrcomp_sandbox_release
if [ "$platform" = windows-x64 ]; then archive="rusty_v8_${profile}_${target}.lib.gz"; else archive="librusty_v8_${profile}_${target}.a.gz"; fi
binding="src_binding_${profile}_${target}.rs"
sums="rusty_v8_${profile}_${target}.sha256"
base="https://github.com/openai/codex/releases/download/rusty-v8-v$v8"
for f in "$sums" "$archive" "$binding"; do
  [ -s "$v8_dir/$f" ] || curl -fsSL --retry 3 -o "$v8_dir/$f" "$base/$f"
done
pinned="third_party/v8/rusty_v8_${v8//./_}_release_manifests.sha256"
expected="$(grep -F "  $sums" "$pinned" | cut -d' ' -f1)"
[ -n "$expected" ] && [ "$(sha256 "$v8_dir/$sums")" = "$expected" ] || { echo "V8 checksum file does not match $pinned" >&2; exit 1; }
(cd "$v8_dir" && tr -d '\r' < "$sums" | sha256sum --check --quiet -) || { echo "V8 download checksum mismatch" >&2; exit 1; }
export RUSTY_V8_ARCHIVE="$(winpath "$v8_dir/$archive")" RUSTY_V8_SRC_BINDING_PATH="$(winpath "$v8_dir/$binding")"

# 3. No build paths in the binaries: rustc remaps the source and cargo paths, C / C++ code compiled by
#    build scripts gets the same through CFLAGS / CXXFLAGS. Windows: the PDB path recorded in the exe is
#    the file name only. The target-specific variable adds to .cargo/config.toml's rustflags (cargo joins
#    target.<triple> and target.<cfg> flags) instead of replacing them, as RUSTFLAGS would.
tvar="CARGO_TARGET_$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')_RUSTFLAGS"
sysroot="$(cd codex-rs && rustc --print sysroot)"   # the pinned toolchain; its rust-src paths too
tdir="$(unixpath "${CARGO_TARGET_DIR:-$root/codex-rs/target}")"
mkdir -p "$tdir"
tdir="$(cd "$tdir" && pwd -P)"
export CARGO_TARGET_DIR="$(winpath "$tdir")"
remap="--remap-path-prefix=$(winpath "$tdir")=/target --remap-path-prefix=$(winpath "$root")=/cute-codex --remap-path-prefix=$(winpath "$cargo_home")=/cargo --remap-path-prefix=$(winpath "$v8_dir")=/v8 --remap-path-prefix=$sysroot=/rust"
if [ "$platform" = windows-x64 ]; then
  export "$tvar=$remap -Clink-arg=/PDBALTPATH:%_PDB%"
  trim="/d1trimfile:$(winpath "$root")\\ /d1trimfile:$(winpath "$tdir")\\ /d1trimfile:$(winpath "$cargo_home")\\"
  export CFLAGS="${CFLAGS:+$CFLAGS }$trim" CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$trim"
else
  export "$tvar=$remap"
  map="-ffile-prefix-map=$tdir=/target -ffile-prefix-map=$root=/cute-codex -ffile-prefix-map=$cargo_home=/cargo -ffile-prefix-map=$v8_dir=/v8"
  export CFLAGS="${CFLAGS:+$CFLAGS }$map" CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$map"
fi
export CARGO_NET_GIT_FETCH_WITH_CLI=true

# 4. Build (the same two cargo runs as the from-source install; the toolchain comes from
#    codex-rs/rust-toolchain.toml).
cd codex-rs
cargo fetch --locked
cargo build --locked --release -p codex-cli -p codex-app-server --bin codex --bin codex-app-server
cargo build --locked --release -p codex-code-mode-host --bin codex-code-mode-host
version="$("$tdir/release/codex-app-server$exe" --version | awk '{ print $2 }')"
case "$version" in
  [0-9]*) ;;
  *) echo "unexpected codex-app-server --version output: $version" >&2; exit 1 ;;
esac

# 5. Stage: stripped binaries (Linux; Windows keeps its PDBs out of the archive) + schema + build record.
stage="$root/../native-stage-$platform"
rm -rf "${stage:?}"
mkdir -p "$stage"
for b in codex-app-server codex codex-code-mode-host; do
  if [ "$platform" = linux-x64 ]; then
    strip -o "$stage/$b" "$tdir/release/$b"
  else
    cp "$tdir/release/$b$exe" "$stage/$b$exe"
  fi
done
cp app-server-protocol/schema/json/codex_app_server_protocol.schemas.json "$stage/"
# License files: the repository's LICENSE (Apache-2.0) and NOTICE, and the notices of every third-party
# crate compiled into the three binaries (from Cargo.lock and the crate sources the build downloaded; no
# network, deterministic).
for f in LICENSE NOTICE; do cp "$root/$f" "$stage/$f"; done
if [ "$platform" = windows-x64 ]; then py=python; else py=python3; fi
"$py" "$root/.github/cute-codex/third_party_notices.py" --manifest-path "$(winpath "$root/codex-rs/Cargo.toml")" \
  --target "$target" --package codex-cli --package codex-app-server --package codex-code-mode-host \
  --title "cute-codex: third-party notices" \
  --intro "codex, codex-app-server and codex-code-mode-host in this archive are cute-codex, a modified version of OpenAI Codex, licensed under the Apache License 2.0 (LICENSE; attributions in NOTICE). They are built from the third-party Rust crates below. codex-code-mode-host also links the prebuilt V8 library of the v8 crate (rusty_v8), whose published crate ships no license files: see https://github.com/denoland/rusty_v8 for V8's license and those of the libraries it bundles." \
  --out "$(winpath "$stage/THIRD_PARTY_NOTICES.md")"
chmod 0644 "$stage/LICENSE" "$stage/NOTICE" "$stage/THIRD_PARTY_NOTICES.md"
rustc_version="$(rustc --version)"
cat > "$stage/cute-codex-build.json" <<EOF
{
  "version": "$version",
  "commit": "$commit",
  "platform": "$platform",
  "target": "$target",
  "rustc": "$rustc_version",
  "v8": "$v8"
}
EOF

# 6. Leak check: no absolute build path (the checkout, cargo's home, the V8 download, the toolchain,
#    the user's home).
leaks=0
for f in "$stage"/*; do
  for p in $(for d in "$root" "$tdir" "$cargo_home" "$v8_dir" "$sysroot" "$HOME"; do winpath "$d"; echo; mixpath "$d"; echo; echo "$d"; done | sort -u); do
    [ -n "$p" ] && [ "$p" != / ] || continue
    if grep -qaF -- "$p" "$f"; then
      echo "::warning::$(basename "$f") contains the build path $p ($(grep -caF -- "$p" "$f") lines)"
      leaks=1
    fi
  done
done
if [ "$leaks" = 1 ] && [ "${LEAK_POLICY:-fail}" = fail ]; then
  echo "build paths found in the binaries (LEAK_POLICY=fail)" >&2
  exit 1
fi

# 7. Pack: ustar, files only, sorted, fixed owner and mtime (the commit time), gzip without a timestamp.
name="native-$version-$platform.tar.gz"
mtime="$(git -C "$root" log -1 --format=%ct)"
files=(codex-app-server$exe codex$exe codex-code-mode-host$exe codex_app_server_protocol.schemas.json cute-codex-build.json
  LICENSE NOTICE THIRD_PARTY_NOTICES.md)
tar -C "$stage" --format=ustar --owner=0 --group=0 --numeric-owner --mtime="@$mtime" -cf - "${files[@]}" | gzip -n -9 > "$out/$name"
cp "$stage/cute-codex-build.json" "$out/BUILD-INFO-$platform.json"
(cd "$out" && sha256sum "$name" > "SHA256SUMS-$platform")
echo "built $out/$name ($version, ${commit:0:12})"
cat "$out/SHA256SUMS-$platform"
