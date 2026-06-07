#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${workspace_dir}"

cargo build --package web_server --features dev-login
systemctl --user restart web_server-debug.service
