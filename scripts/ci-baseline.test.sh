#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
baseline="${script_dir}/ci-baseline.sh"
work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}"' EXIT

# A stub gh prints GH_STUB_SHA as the newest successful run's commit, or fails
# when GH_STUB_FAIL is set, and records its arguments.
mkdir -p "${work_dir}/bin"
cat >"${work_dir}/bin/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >"${GH_STUB_ARGS}"
[[ -z "${GH_STUB_FAIL:-}" ]] || exit 1
printf '%s\n' "${GH_STUB_SHA:-}"
STUB
chmod +x "${work_dir}/bin/gh"
export GH_STUB_ARGS="${work_dir}/gh-args"

# base -> verified -> head on main, plus a commit on a diverged line.
repo="${work_dir}/repo"
git init -q "${repo}"
cd "${repo}"
git config user.email test@example.com
git config user.name test
git commit -q --allow-empty -m base
base_commit="$(git rev-parse HEAD)"
git commit -q --allow-empty -m verified
verified="$(git rev-parse HEAD)"
git commit -q --allow-empty -m head
head="$(git rev-parse HEAD)"
git checkout -q -b diverged "${base_commit}"
git commit -q --allow-empty -m diverged
diverged="$(git rev-parse HEAD)"

resolve() {
  GH_STUB_SHA="$1" GH_STUB_FAIL="${2:-}" PATH="${work_dir}/bin:${PATH}" \
    "${baseline}" deploy-staging.yml main "${head}" 2>/dev/null
}

fail() {
  echo "ci-baseline: $*" >&2
  exit 1
}

[[ "$(resolve "${verified}")" == "${verified}" ]] \
  || fail "a verified ancestor of HEAD must be the baseline"
[[ "$(cat "${GH_STUB_ARGS}")" == "run list --workflow deploy-staging.yml --branch main --event push --status success --limit 1 --json headSha --jq .[0].headSha // empty" ]] \
  || fail "unexpected gh query: $(cat "${GH_STUB_ARGS}")"
[[ "$(resolve "${head}")" == "${head}" ]] \
  || fail "a re-run of an already verified HEAD diffs from itself"
[[ -z "$(resolve "${diverged}")" ]] \
  || fail "a commit that is not an ancestor of HEAD (force push) must not be the baseline"
[[ -z "$(resolve "")" ]] \
  || fail "no successful run must select every suite"
[[ -z "$(resolve "${verified}" 1)" ]] \
  || fail "a failed run lookup must select every suite"
[[ -z "$(resolve "not-a-sha")" ]] \
  || fail "a malformed commit must not be the baseline"
[[ -z "$(resolve "0123456789abcdef0123456789abcdef01234567")" ]] \
  || fail "a commit missing from the checkout must not be the baseline"

echo "ci-baseline tests passed"
