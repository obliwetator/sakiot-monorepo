#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${workspace_dir}"

cargo build --package fbi_agent
systemctl --user restart fbi-agent-debug.service
