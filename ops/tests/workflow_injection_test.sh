#!/usr/bin/env bash
# GitHub expressions (${{ ... }}) are expanded into the script text before bash
# parses it. A branch name or workflow input containing quotes can therefore
# close the quoting and run arbitrary commands on the runner. Expressions must
# be passed through `env:` and referenced as shell variables instead.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${test_dir}/../.." && pwd)"
workflows="${repo_root}/.github/workflows"

# 1. No expression may appear inside a `run:` block in any workflow.
offenders="$(
  awk '
    /^[[:space:]]*(-[[:space:]]+)?run:[[:space:]]*[|>]/ {
      in_run = 1
      base = match($0, /[^ ]/) - 1
      next
    }
    in_run {
      if ($0 ~ /^[[:space:]]*$/) next
      indent = match($0, /[^ ]/) - 1
      if (indent <= base) { in_run = 0; next }
      if ($0 ~ /\$\{\{/) { print FILENAME ":" FNR ": " $0; bad = 1 }
    }
    # Single-line scripts: `run: echo ${{ github.event.ref }}`.
    /^[[:space:]]*(-[[:space:]]+)?run:[[:space:]]+[^|>[:space:]]/ {
      if ($0 ~ /\$\{\{/) { print FILENAME ":" FNR ": " $0; bad = 1 }
    }
    END { exit bad ? 1 : 0 }
  ' "${workflows}"/*.yml || true
)"
if [[ -n "${offenders}" ]]; then
  echo "GitHub expressions interpolated into a run script:" >&2
  printf '%s\n' "${offenders}" >&2
  exit 1
fi

# 2. The preview slot derivation must treat a hostile branch name as data.
derive_script="$(
  awk '
    /id: derive/ { found = 1 }
    found && /run: \|/ { capture = 1; next }
    capture {
      if ($0 ~ /^[[:space:]]*$/) { print; next }
      if ($0 ~ /^        /) { sub(/^        /, ""); print; next }
      exit
    }
  ' "${workflows}/deploy-preview.yml"
)"
if [[ -z "${derive_script}" ]]; then
  echo "could not extract the preview slot derivation script" >&2
  exit 1
fi

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
marker="${temporary}/injected"

run_derive() {
  GITHUB_EVENT_NAME="$1" \
    EVENT_REF="$2" \
    INPUT_BRANCH="$3" \
    INPUT_SLOT="$4" \
    GITHUB_OUTPUT="$5" \
    bash -c "${derive_script}"
}

# A branch name that would close the quoting in an interpolated script.
hostile="x\"; touch ${marker}; #"

output="${temporary}/output-delete"
run_derive delete "${hostile}" "" "" "${output}" >/dev/null
if [[ -e "${marker}" ]]; then
  echo "hostile branch name executed as shell code" >&2
  exit 1
fi
# It must be slugged like any other data: only [a-z0-9-] survives.
grep -q '^slot=x-touch-tmp-' "${output}" \
  || { echo "unexpected slot for hostile branch: $(cat "${output}")" >&2; exit 1; }

# A dispatch slot is taken verbatim, so a hostile one must be rejected instead
# of reaching the shell or the deploy.
output="${temporary}/output-dispatch"
if run_derive workflow_dispatch "" "${hostile}" "${hostile}" "${output}" \
  >/dev/null 2>&1; then
  echo "hostile dispatch input must be rejected" >&2
  exit 1
fi
if [[ -e "${marker}" ]]; then
  echo "hostile dispatch input executed as shell code" >&2
  exit 1
fi

# A normal branch still derives its slot.
output="${temporary}/output-normal"
run_derive delete "refs/heads/Feature_Clip" "" "" "${output}" >/dev/null
grep -qx 'slot=refs-heads-feature-clip' "${output}" \
  || { echo "unexpected slot for a normal branch: $(cat "${output}")" >&2; exit 1; }
[[ ! -e "${marker}" ]]

echo "workflow injection guard: ok"
