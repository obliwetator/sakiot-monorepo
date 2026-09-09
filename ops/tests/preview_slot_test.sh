#!/usr/bin/env bash
# `--remove` must finish the teardown even when Cloudflare is unreachable:
# leaving the vhost, units, database, and data behind leaks far more than a
# stale DNS record, so DNS is best-effort and only affects the exit status.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${test_dir}/../.." && pwd)"
script="${repo_root}/ops/preview-slot.sh"

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

write_stubs() {
  local dir="$1"
  mkdir -p "${dir}"

  # Pretend to be root so the script's root check passes without sudo.
  cat >"${dir}/id" <<'EOF'
#!/usr/bin/env bash
printf '0\n'
EOF

  # Record every unit operation so the test can prove cleanup ran.
  cat >"${dir}/systemctl" <<EOF
#!/usr/bin/env bash
printf 'systemctl %s\n' "\$*" >>"${temporary}/systemctl.log"
EOF

  cat >"${dir}/sudo" <<'EOF'
#!/usr/bin/env bash
# Only the postgres probes are reached during teardown; report "no database".
exit 1
EOF

  cat >"${dir}/nginx" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

  cat >"${dir}/certbot" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

  chmod +x "${dir}"/*
}

write_stubs "${temporary}/bin-fail"
# Cloudflare is unreachable.
cat >"${temporary}/bin-fail/curl" <<'EOF'
#!/usr/bin/env bash
exit 22
EOF
cat >"${temporary}/bin-fail/jq" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf '\n'
EOF
chmod +x "${temporary}/bin-fail/curl" "${temporary}/bin-fail/jq"

write_stubs "${temporary}/bin-ok"
# Cloudflare answers with a zone and a record.
cat >"${temporary}/bin-ok/curl" <<'EOF'
#!/usr/bin/env bash
for arg in "$@"; do
  [[ "$arg" == "DELETE" ]] && exit 0
  case "$arg" in
    *"/zones?name="*) printf '{"result":[{"id":"zone-1"}]}'; exit 0 ;;
    *"/dns_records?type=A"*) printf '{"result":[{"id":"record-1"}]}'; exit 0 ;;
  esac
done
exit 22
EOF
cat >"${temporary}/bin-ok/jq" <<'EOF'
#!/usr/bin/env bash
input="$(cat)"
printf '%s' "${input}" | grep -o '"id":"[^"]*"' | head -n1 | cut -d'"' -f4
EOF
chmod +x "${temporary}/bin-ok/curl" "${temporary}/bin-ok/jq"

state_root="${temporary}/state"

# Fixture files for every per-slot root the teardown must remove. The script
# is pointed at this prefix so the test never touches real slot state.
seed_slot_roots() {
  rm -rf "${state_root}"
  mkdir -p \
    "${state_root}/var/lib/sakiot-preview-zz-ops-test/data" \
    "${state_root}/var/lib/sakiot-preview-zz-ops-test/deploy" \
    "${state_root}/var/lib/sakiot-preview-zz-ops-test/backups" \
    "${state_root}/srv/sakiot-preview-zz-ops-test/releases/v1" \
    "${state_root}/var/cache/sakiot-preview-zz-ops-test" \
    "${state_root}/var/www/zz-ops-test.preview.example.test"
  : >"${state_root}/var/lib/sakiot-preview-zz-ops-test/data/recording.ogg"
  : >"${state_root}/var/lib/sakiot-preview-zz-ops-test/deploy/current"
  : >"${state_root}/var/lib/sakiot-preview-zz-ops-test/backups/old.dump"
  : >"${state_root}/srv/sakiot-preview-zz-ops-test/releases/v1/binary"
  : >"${state_root}/var/cache/sakiot-preview-zz-ops-test/cache.bin"
  : >"${state_root}/var/www/zz-ops-test.preview.example.test/index.html"
}

assert_slot_roots_removed() {
  # The shared parents stay; every slot-specific path must be gone.
  for path in \
    "var/lib/sakiot-preview-zz-ops-test" \
    "srv/sakiot-preview-zz-ops-test" \
    "var/cache/sakiot-preview-zz-ops-test" \
    "var/www/zz-ops-test.preview.example.test"; do
    [[ ! -e "${state_root}/${path}" ]] \
      || { echo "leftover per-slot root: ${path}" >&2; exit 1; }
  done
}

run_remove() {
  local bin_dir="$1" token="$2" output="$3"
  seed_slot_roots
  set +e
  PATH="${bin_dir}:${PATH}" \
    CLOUDFLARE_API_TOKEN="${token}" \
    PREVIEW_DOMAIN="preview.example.test" \
    SAKIOT_PREVIEW_STATE_ROOT="${state_root}" \
    "${script}" zz-ops-test --remove >"${output}" 2>&1
  local status=$?
  set -e
  printf '%s' "${status}"
}

assert_cleanup_ran() {
  local output="$1"
  grep -q "removed nginx vhost zz-ops-test.preview.example.test" "${output}" \
    || { echo "nginx vhost was not removed: $(cat "${output}")" >&2; exit 1; }
  grep -q "removed systemd units" "${output}" \
    || { echo "systemd units were not removed: $(cat "${output}")" >&2; exit 1; }
  grep -qx "systemctl daemon-reload" "${temporary}/systemctl.log" \
    || { echo "systemctl daemon-reload never ran" >&2; exit 1; }
  grep -q "removed per-slot roots" "${output}" \
    || { echo "per-slot roots were not removed: $(cat "${output}")" >&2; exit 1; }
  assert_slot_roots_removed
}

# 1. Cloudflare unreachable: cleanup still runs, exit is non-zero.
output="${temporary}/fail.log"
status="$(run_remove "${temporary}/bin-fail" "test-token" "${output}")"
[[ "${status}" -ne 0 ]] \
  || { echo "a failed DNS deletion must exit non-zero" >&2; exit 1; }
grep -q "could not look up the Cloudflare zone" "${output}" \
  || { echo "missing DNS failure warning: $(cat "${output}")" >&2; exit 1; }
assert_cleanup_ran "${output}"
grep -q "slot zz-ops-test removed (with DNS warnings)" "${output}" \
  || { echo "missing final warning: $(cat "${output}")" >&2; exit 1; }

# 2. No token at all: same guarantee.
output="${temporary}/no-token.log"
status="$(run_remove "${temporary}/bin-fail" "" "${output}")"
[[ "${status}" -ne 0 ]] \
  || { echo "a missing token must exit non-zero" >&2; exit 1; }
grep -q "CLOUDFLARE_API_TOKEN is not set" "${output}" \
  || { echo "missing token warning: $(cat "${output}")" >&2; exit 1; }
assert_cleanup_ran "${output}"

# 3. DNS deleted: cleanup runs and the exit status stays zero.
: >"${temporary}/systemctl.log"
output="${temporary}/ok.log"
status="$(run_remove "${temporary}/bin-ok" "test-token" "${output}")"
[[ "${status}" -eq 0 ]] \
  || { echo "a successful removal must exit zero: $(cat "${output}")" >&2; exit 1; }
grep -q "deleted DNS record zz-ops-test.preview.example.test" "${output}" \
  || { echo "missing DNS deletion log: $(cat "${output}")" >&2; exit 1; }
assert_cleanup_ran "${output}"
grep -q "slot zz-ops-test removed$" "${output}" \
  || { echo "missing success log: $(cat "${output}")" >&2; exit 1; }

echo "preview slot removal: ok"
