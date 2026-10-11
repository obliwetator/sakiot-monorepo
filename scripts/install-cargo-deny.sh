#!/usr/bin/env bash
# Install the pinned cargo-deny release into Cargo's bin directory, verified
# against the committed checksum, for CI.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(tr -d '[:space:]' <"${repo_root}/scripts/cargo-deny-version")"
sha256="$(tr -d '[:space:]' <"${repo_root}/scripts/cargo-deny-linux-sha256")"
name="cargo-deny-${version}-x86_64-unknown-linux-musl"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
curl -LsSf -o "${work}/${name}.tar.gz" \
  "https://github.com/EmbarkStudios/cargo-deny/releases/download/${version}/${name}.tar.gz"
echo "${sha256}  ${work}/${name}.tar.gz" | sha256sum -c -
tar xzf "${work}/${name}.tar.gz" -C "${work}"
install -m 0755 "${work}/${name}/cargo-deny" "${CARGO_HOME:-$HOME/.cargo}/bin/cargo-deny"
cargo deny --version
