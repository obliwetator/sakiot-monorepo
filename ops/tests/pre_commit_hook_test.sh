#!/usr/bin/env bash
set -euo pipefail

# The pre-commit hook formats staged content and writes it back to the index.
# Stub formatters stand in for rustfmt and Biome: they rewrite UNFORMATTED to
# formatted and reject content containing SYNTAX_ERROR, so this suite needs
# neither the Rust toolchain nor Bun.

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
hook="${test_dir}/../../.githooks/pre-commit"
work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}"' EXIT

stub_bin="${work_dir}/bin"
mkdir -p "${stub_bin}"
cat >"${stub_bin}/rustfmt" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$#" -eq 2 && "$1" == "--edition" && "$2" == "2024" ]] || {
  echo "rustfmt stub: unexpected arguments: $*" >&2
  exit 2
}
content="$(cat; printf x)"
content="${content%x}"
[[ "${content}" != *SYNTAX_ERROR* ]] || { echo "rustfmt stub: parse error" >&2; exit 1; }
printf '%s' "${content//UNFORMATTED/formatted}"
STUB
cat >"${stub_bin}/bunx" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ "$#" -eq 4 && "$1 $2 $3" == "biome check --write" && "$4" == --stdin-file-path=src/* ]] || {
  echo "bunx stub: unexpected arguments: $*" >&2
  exit 2
}
[[ "$(basename "$(pwd)")" == "sakiot-stage" ]] || { echo "bunx stub: wrong cwd" >&2; exit 2; }
content="$(cat; printf x)"
content="${content%x}"
[[ "${content}" != *SYNTAX_ERROR* ]] || { echo "bunx stub: parse error" >&2; exit 1; }
printf '%s' "${content//UNFORMATTED/formatted}"
STUB
chmod +x "${stub_bin}/rustfmt" "${stub_bin}/bunx"

repo="${work_dir}/repo"
mkdir -p "${repo}/.githooks" "${repo}/crate/src" "${repo}/sakiot-stage/src"
cp "${hook}" "${repo}/.githooks/pre-commit"
cd "${repo}"
git init -q
git config user.email test@example.com
git config user.name test
printf '[workspace]\nmembers = ["crate"]\n' >Cargo.toml
printf '[package]\nname = "crate"\nedition = "2024"\n' >crate/Cargo.toml
printf 'line 1\nline 2\nline 3\nline 4\nline 5\n' >crate/src/lib.rs
printf 'export const a = 1;\n' >sakiot-stage/src/app.ts
printf 'notes\n' >README.md
git add -A
git commit -q -m base

run_hook() {
  PATH="${stub_bin}:${PATH}" sh .githooks/pre-commit >/dev/null
}

fail() {
  echo "pre-commit hook: $*" >&2
  exit 1
}

# A partially staged file commits only its staged hunk, formatted, and keeps
# its unstaged hunk unstaged.
printf 'line 1 UNFORMATTED\nline 2\nline 3\nline 4\nline 5\n' >crate/src/lib.rs
git add crate/src/lib.rs
printf 'line 1 UNFORMATTED\nline 2\nline 3\nline 4\nline 5 wip UNFORMATTED\n' >crate/src/lib.rs
run_hook
[[ "$(git show :crate/src/lib.rs)" == $'line 1 formatted\nline 2\nline 3\nline 4\nline 5' ]] \
  || fail "the staged hunk was not formatted, or the unstaged hunk was staged"
[[ "$(cat crate/src/lib.rs)" == $'line 1 formatted\nline 2\nline 3\nline 4\nline 5 wip formatted' ]] \
  || fail "the working tree lost its unstaged hunk or was not formatted"
[[ "$(git diff --name-only)" == "crate/src/lib.rs" ]] \
  || fail "the unstaged hunk should remain the only unstaged change"
git commit -q --no-verify -m partial
git checkout -q -- crate/src/lib.rs

# A fully staged frontend file is formatted in the index and on disk alike.
printf 'export const a = UNFORMATTED;\n' >sakiot-stage/src/app.ts
git add sakiot-stage/src/app.ts
run_hook
[[ "$(git show :sakiot-stage/src/app.ts)" == "export const a = formatted;" ]] \
  || fail "the staged frontend file was not formatted"
git diff --quiet -- sakiot-stage/src/app.ts \
  || fail "the working tree should match the formatted index"
git commit -q --no-verify -m frontend

# Files the formatters do not own are left untouched.
printf 'UNFORMATTED notes\n' >README.md
git add README.md
run_hook
[[ "$(git show :README.md)" == "UNFORMATTED notes" ]] \
  || fail "a Markdown file was passed to a formatter"
git commit -q --no-verify -m notes

# Staged content that does not parse aborts the commit.
printf 'SYNTAX_ERROR\n' >crate/src/lib.rs
git add crate/src/lib.rs
if run_hook 2>/dev/null; then
  fail "accepted staged content its formatter could not parse"
fi
git reset -q --hard

# Unstaged edits that do not parse do not block formatting the staged hunk.
printf 'line 1 UNFORMATTED\nline 2\nline 3\nline 4\nline 5\n' >crate/src/lib.rs
git add crate/src/lib.rs
printf 'line 1 UNFORMATTED\nline 2\nline 3\nline 4\nline 5 SYNTAX_ERROR\n' >crate/src/lib.rs
run_hook 2>/dev/null
[[ "$(git show :crate/src/lib.rs)" == $'line 1 formatted\nline 2\nline 3\nline 4\nline 5' ]] \
  || fail "an unparsable unstaged hunk blocked formatting the staged one"
[[ "$(sed -n 5p crate/src/lib.rs)" == "line 5 SYNTAX_ERROR" ]] \
  || fail "the unparsable unstaged hunk was modified"
