#!/usr/bin/env bash
# Points an env file's DATABASE_URL and BACKUP_DATABASE_URL at another
# PostgreSQL role, keeping host, database, and query, and quoted or not. The
# libpq defaults PGUSER and PGPASSWORD, if set, follow. The previous file is kept
# next to it as <file>.bak-<timestamp>, with the same owner and mode, unless
# --no-backup is given (for copies whose old credentials should not linger).
#
#   ops/rewrite-db-role.sh [--no-backup] <env file> <role> <password>
#
# The password must be URL-safe (letters, digits, - . _ ~) so it can be written
# into the URL verbatim.
set -euo pipefail

backup=1
if [[ "${1:-}" == "--no-backup" ]]; then
  backup=0
  shift
fi
if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 [--no-backup] <env file> <role> <password>" >&2
  exit 2
fi
env_file="$1"
role="$2"
password="$3"
[[ -f "${env_file}" ]] || { echo "no such env file: ${env_file}" >&2; exit 1; }
[[ "${role}" =~ ^[a-z_][a-z0-9_]*$ ]] || { echo "invalid role '${role}'" >&2; exit 2; }
[[ "${password}" =~ ^[A-Za-z0-9._~-]+$ ]] || { echo "password must be URL-safe" >&2; exit 2; }

rewritten="$(mktemp "${env_file}.XXXXXX")"
trap 'rm -f "${rewritten}"' EXIT
awk -v role="${role}" -v password="${password}" '
  /^(BACKUP_)?DATABASE_URL=["\047]?postgres(ql)?:\/\/[^\/@]*@/ {
    prefix = $0
    sub(/:\/\/.*/, "://", prefix)
    rest = $0
    sub(/^[^:]*:\/\/[^\/@]*@/, "", rest)
    print prefix role ":" password "@" rest
    changed++
    next
  }
  # Keep the quote style: a trailing quote closes the leading one.
  /^PG(USER|PASSWORD)=/ {
    name = $0
    sub(/=.*/, "", name)
    value = substr($0, length(name) + 2)
    quote = substr(value, 1, 1)
    if (quote != "\"" && quote != "\047") quote = ""
    print name "=" quote (name == "PGUSER" ? role : password) quote
    next
  }
  { print }
  END { if (changed == 0) exit 3 }
' "${env_file}" >"${rewritten}" || {
  echo "no DATABASE_URL with credentials found in ${env_file}" >&2
  exit 1
}

chown --reference="${env_file}" "${rewritten}" 2>/dev/null || true
chmod --reference="${env_file}" "${rewritten}"
if [[ "${backup}" -eq 1 ]]; then
  backup_file="${env_file}.bak-$(date +%Y%m%d%H%M%S)"
  cp -p "${env_file}" "${backup_file}"
  mv "${rewritten}" "${env_file}"
  trap - EXIT
  echo "${env_file}: database role set to ${role} (previous version in ${backup_file})"
else
  mv "${rewritten}" "${env_file}"
  trap - EXIT
  echo "${env_file}: database role set to ${role}"
fi
