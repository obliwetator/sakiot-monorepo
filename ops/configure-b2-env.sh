#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: sudo bash ops/configure-b2-env.sh \
  PROD_KEY_FILE STAGING_KEY_FILE PROD_BUCKET STAGING_BUCKET BACKUP_BUCKET

Key files must each contain exactly: APPLICATION_KEY_ID APPLICATION_KEY
EOF
  exit 2
}

[[ $# -eq 5 ]] || usage
[[ ${EUID} -eq 0 ]] || { echo "run as root" >&2; exit 1; }

prod_key_file=$1
staging_key_file=$2
prod_bucket=$3
staging_bucket=$4
backup_bucket=$5

prod_env=/etc/sakiot/production.env
staging_env=/etc/sakiot/staging.env
endpoint=https://s3.eu-central-003.backblazeb2.com
region=eu-central-003

for path in "$prod_key_file" "$staging_key_file" "$prod_env" "$staging_env"; do
  [[ -f $path && ! -L $path ]] || { echo "missing or unsafe file: $path" >&2; exit 1; }
done

for bucket in "$prod_bucket" "$staging_bucket" "$backup_bucket"; do
  [[ $bucket =~ ^[a-zA-Z0-9-]{6,63}$ ]] || { echo "invalid bucket name: $bucket" >&2; exit 1; }
done

IFS=$' \t' read -r prod_key_id prod_secret prod_extra <"$prod_key_file"
IFS=$' \t' read -r staging_key_id staging_secret staging_extra <"$staging_key_file"

for value in "$prod_key_id" "$staging_key_id"; do
  [[ $value =~ ^[a-zA-Z0-9]+$ ]] || { echo "invalid key-file content" >&2; exit 1; }
done
for value in "$prod_secret" "$staging_secret"; do
  [[ $value =~ ^[a-zA-Z0-9_+/-]+$ ]] || { echo "invalid key-file content" >&2; exit 1; }
done
[[ -z ${prod_extra:-} && -z ${staging_extra:-} ]] || {
  echo "key files must contain exactly two fields" >&2
  exit 1
}

umask 077
temporary_files=()
cleanup() {
  rm -f -- "${temporary_files[@]}"
}
trap cleanup EXIT

rewrite_env() {
  local target=$1
  local bucket=$2
  local key_id=$3
  local secret=$4
  local include_backup=$5
  local pattern temporary

  pattern='^(export[[:space:]]+)?(SAKIOT_MEDIA_ARCHIVE_ENABLED|SAKIOT_MEDIA_S3_ENDPOINT|SAKIOT_MEDIA_S3_REGION|SAKIOT_MEDIA_S3_BUCKET|SAKIOT_MEDIA_S3_ACCESS_KEY_ID|SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY|SAKIOT_MEDIA_LOCAL_RETENTION_DAYS|SAKIOT_MEDIA_LOCAL_CACHE_MAX_BYTES|SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED)='
  if [[ $include_backup == 1 ]]; then
    pattern='^(export[[:space:]]+)?(RCLONE_CONFIG|B2_BACKUP_REMOTE|SAKIOT_MEDIA_ARCHIVE_ENABLED|SAKIOT_MEDIA_S3_ENDPOINT|SAKIOT_MEDIA_S3_REGION|SAKIOT_MEDIA_S3_BUCKET|SAKIOT_MEDIA_S3_ACCESS_KEY_ID|SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY|SAKIOT_MEDIA_LOCAL_RETENTION_DAYS|SAKIOT_MEDIA_LOCAL_CACHE_MAX_BYTES|SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED)='
  fi

  temporary=$(mktemp "${target}.tmp.XXXXXX")
  temporary_files+=("$temporary")
  awk -v pattern="$pattern" '$0 !~ pattern' "$target" >"$temporary"

  {
    printf '\nSAKIOT_MEDIA_ARCHIVE_ENABLED=1\n'
    printf 'SAKIOT_MEDIA_S3_ENDPOINT=%s\n' "$endpoint"
    printf 'SAKIOT_MEDIA_S3_REGION=%s\n' "$region"
    printf 'SAKIOT_MEDIA_S3_BUCKET=%s\n' "$bucket"
    printf 'SAKIOT_MEDIA_S3_ACCESS_KEY_ID=%s\n' "$key_id"
    printf 'SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY=%s\n' "$secret"
    printf 'SAKIOT_MEDIA_LOCAL_RETENTION_DAYS=7\n'
    printf 'SAKIOT_MEDIA_LOCAL_CACHE_MAX_BYTES=53687091200\n'
    printf 'SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=0\n'
    if [[ $include_backup == 1 ]]; then
      printf 'RCLONE_CONFIG=/etc/sakiot/rclone.conf\n'
      printf 'B2_BACKUP_REMOTE=b2:%s\n' "$backup_bucket"
    fi
  } >>"$temporary"

  chown root:sakiot "$temporary"
  chmod 0640 "$temporary"
  mv -f -- "$temporary" "$target"
}

rewrite_env "$prod_env" "$prod_bucket" "$prod_key_id" "$prod_secret" 1
rewrite_env "$staging_env" "$staging_bucket" "$staging_key_id" "$staging_secret" 0

unset prod_secret staging_secret
echo "configured production media bucket: $prod_bucket"
echo "configured staging media bucket: $staging_bucket"
echo "configured backup remote: b2:$backup_bucket"
echo "local pruning remains disabled"
