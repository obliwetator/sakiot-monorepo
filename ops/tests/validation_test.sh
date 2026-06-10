#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/common.sh
source "${test_dir}/../lib/common.sh"

validate_tag v1.2.3
validate_sha 0123456789abcdef0123456789abcdef01234567

if (validate_tag 'release-1') >/dev/null 2>&1; then
  echo "accepted invalid release tag" >&2
  exit 1
fi
if (validate_tag 'v1.0;id') >/dev/null 2>&1; then
  echo "accepted shell metacharacters in tag" >&2
  exit 1
fi
for bad_tag in 'v1.23' 'v1.2.3-rc1' 'v1.2' 'v1.2.3.4' 'v1.2.x'; do
  if (validate_tag "${bad_tag}") >/dev/null 2>&1; then
    echo "accepted non-strict-semver tag: ${bad_tag}" >&2
    exit 1
  fi
done
if (validate_sha 'deadbeef') >/dev/null 2>&1; then
  echo "accepted short commit SHA" >&2
  exit 1
fi

validate_test_database_url \
  'postgres://app:secret@127.0.0.1/sakiot_rouvas' \
  'postgres://tests:secret@127.0.0.1/sakiot_test'
validate_test_database_url \
  'postgresql://app:secret@127.0.0.1/sakiot_rouvas?sslmode=disable' \
  'postgresql://tests:secret@127.0.0.1/sakiot_test?sslmode=disable'
if (validate_test_database_url \
  'postgres://app@127.0.0.1/sakiot_rouvas' \
  'postgres://tests@127.0.0.1/sakiot_rouvas') >/dev/null 2>&1; then
  echo "accepted runtime database as test database" >&2
  exit 1
fi
if (validate_test_database_url \
  'postgres://app@127.0.0.1/sakiot_rouvas' \
  'postgres://tests@127.0.0.1/sakiot_ci') >/dev/null 2>&1; then
  echo "accepted test database without _test suffix" >&2
  exit 1
fi
if (validate_test_database_url \
  'postgres://app@127.0.0.1/sakiot_rouvas' '') >/dev/null 2>&1; then
  echo "accepted missing test database URL" >&2
  exit 1
fi

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
sha=0123456789abcdef0123456789abcdef01234567
moved_sha=1123456789abcdef0123456789abcdef01234567
printf '%s\n' "${sha}" >"${temporary}/v1.0.0"

if (validate_tag_record release "${temporary}/v1.0.0" v1.0.0 "${sha}") \
  >/dev/null 2>&1; then
  echo "accepted reused release tag" >&2
  exit 1
fi
if (validate_tag_record release "${temporary}/v1.0.0" v1.0.0 "${moved_sha}") \
  >/dev/null 2>&1; then
  echo "accepted moved release tag" >&2
  exit 1
fi
validate_tag_record rollback "${temporary}/v1.0.0" v1.0.0 "${sha}"
if (validate_tag_record rollback "${temporary}/missing" v2.0.0 "${sha}") \
  >/dev/null 2>&1; then
  echo "accepted rollback tag without successful deployment record" >&2
  exit 1
fi
