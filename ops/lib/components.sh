#!/usr/bin/env bash

all_components() {
  printf '%s\n' database bot web frontend
}

components_for_paths() {
  local path
  local unknown=0
  declare -A selected=()

  if [[ "$#" -eq 0 ]]; then
    all_components
    return
  fi

  for path in "$@"; do
    case "${path}" in
      *.md|docs/*|ops/*)
        # ops/ is the deploy framework, installed out-of-band into
        # /usr/local/lib/sakiot-deploy and /etc/systemd/system, not consumed
        # from the release tag. Tag-time ops/ changes do not rebuild artifacts.
        ;;
      FBI-agent/*)
        selected[bot]=1
        ;;
      web_server/*)
        selected[web]=1
        ;;
      sakiot_stage/*)
        selected[frontend]=1
        ;;
      sakiot-paths/*|sakiot-proto/*|Cargo.toml|Cargo.lock|.sqlx/*)
        selected[bot]=1
        selected[web]=1
        ;;
      sakiot-db/migrations/*)
        selected[database]=1
        selected[bot]=1
        selected[web]=1
        ;;
      sakiot-db/ops/*|.github/*|compose*.yml|compose*.yaml|.env.example)
        selected[bot]=1
        selected[web]=1
        selected[frontend]=1
        ;;
      *)
        unknown=1
        ;;
    esac
  done

  if [[ "${unknown}" == "1" ]]; then
    all_components
    return
  fi

  for path in database bot web frontend; do
    if [[ -n "${selected[${path}]:-}" ]]; then
      printf '%s\n' "${path}"
    fi
  done
}

component_selected() {
  local wanted="$1"
  shift
  local component
  for component in "$@"; do
    [[ "${component}" == "${wanted}" ]] && return 0
  done
  return 1
}
