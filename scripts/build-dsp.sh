#!/usr/bin/env bash
# Build the shared DSP browser artifacts into sakiot-dsp/pkg/.
#
# pkg/ is build output and is not committed. sakiot-stage's dev, test,
# typecheck, and bundle scripts call this first, so every frontend build -
# local, CI, and the deploy engine on the VPS - compiles the WASM from the
# DSP sources in the same checkout. The CI `dsp` job also runs it before
# verifying WASM/native parity, and `bun run wasm:build` in sakiot-dsp
# delegates to it.
#
# Keeping the whole build here means the pinned wasm-bindgen CLI version and
# the bundler are only ever named in one place. Build order matters: the
# worklet bundle inlines pkg/sakiot_dsp.js, so wasm-bindgen must run first.
#
# Requirements: the wasm32-unknown-unknown target (listed in
# rust-toolchain.toml), the wasm-bindgen CLI at the version in
# sakiot-dsp/wasm-bindgen-cli-version, and rolldown (installed by
# `bun install` in sakiot-stage or sakiot-dsp).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dsp_root="${repo_root}/sakiot-dsp"
pkg_root="${dsp_root}/pkg"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command -v cargo >/dev/null 2>&1 \
  || fail "cargo is required to build the shared DSP artifacts"

# Resolve the toolchain the same way the build will: from sakiot-dsp, so the
# repository's rust-toolchain.toml selects the pinned compiler and its targets.
if command -v rustup >/dev/null 2>&1; then
  if ! (cd "${dsp_root}" && rustup target list --installed 2>/dev/null) \
    | grep -qx 'wasm32-unknown-unknown'; then
    fail "missing wasm32-unknown-unknown target; run: rustup target add wasm32-unknown-unknown"
  fi
fi

pinned_bindgen="$(tr -d '[:space:]' <"${dsp_root}/wasm-bindgen-cli-version")"
command -v wasm-bindgen >/dev/null 2>&1 \
  || fail "wasm-bindgen is not installed; run: cargo install wasm-bindgen-cli --version ${pinned_bindgen} --locked"
installed_bindgen="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')"
[[ "${installed_bindgen}" == "${pinned_bindgen}" ]] \
  || fail "wasm-bindgen ${installed_bindgen} does not match the pinned ${pinned_bindgen}; run: cargo install wasm-bindgen-cli --version ${pinned_bindgen} --locked"

# sakiot-dsp and sakiot-stage both pin rolldown. Either install is acceptable
# as long as the version matches, which keeps the bundle byte-identical from
# both trees.
pinned_rolldown="$(sed -n 's/.*"rolldown": *"\([^"]*\)".*/\1/p' "${dsp_root}/package.json" | head -n1)"
rolldown=""
for candidate in \
  "${dsp_root}/node_modules/.bin/rolldown" \
  "${repo_root}/sakiot-stage/node_modules/.bin/rolldown"; do
  if [[ -x "${candidate}" ]]; then
    rolldown="${candidate}"
    break
  fi
done
[[ -n "${rolldown}" ]] \
  || fail "rolldown is not installed; run 'bun install' in sakiot-stage or sakiot-dsp"
installed_rolldown="$("${rolldown}" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+[^ ]*' | head -n1 || true)"
[[ -z "${pinned_rolldown}" || "${installed_rolldown}" == "${pinned_rolldown}" ]] \
  || fail "rolldown ${installed_rolldown} does not match the pinned ${pinned_rolldown}; run 'bun install' in sakiot-stage"

# Every step runs from sakiot-dsp: rustup picks the pinned toolchain from the
# repository's rust-toolchain.toml, and rolldown writes `//#region <path>`
# banners relative to its working directory. Building from anywhere else would
# silently use another toolchain or emit a different bundle.
(
  cd "${dsp_root}"

  cargo build --locked --release --package sakiot-dsp \
    --target wasm32-unknown-unknown --features wasm

  # sakiot-dsp is a root workspace member, so output lands in the workspace
  # target directory. A relative CARGO_TARGET_DIR resolves from this
  # directory, as it does for cargo.
  wasm-bindgen \
    "${CARGO_TARGET_DIR:-${repo_root}/target}/wasm32-unknown-unknown/release/sakiot_dsp.wasm" \
    --out-dir pkg \
    --target web

  "${rolldown}" web/sakiot-dsp-worklet.js \
    --file pkg/sakiot-dsp-worklet.bundle.js \
    --format es
)

# Fail here rather than in a downstream import: these are exactly the paths the
# frontend and the parity verifier resolve.
for artifact in \
  sakiot_dsp.js \
  sakiot_dsp.d.ts \
  sakiot_dsp_bg.wasm \
  sakiot_dsp_bg.wasm.d.ts \
  sakiot-dsp-worklet.bundle.js; do
  [[ -s "${pkg_root}/${artifact}" ]] \
    || fail "missing or empty build output: pkg/${artifact}"
done

printf 'built shared DSP artifacts in %s\n' "${pkg_root}"
