#!/usr/bin/env bash
# What the bot spent in each step of a sakiot-speakers run, from the VPS's
# Prometheus. Reads the run's `window <speakers> <start> <end>` lines.
#
#   ops/sakiot-speakers/measure.sh run.log
#
# SAKIOT_DEV_SSH (from the environment or the repository's .env) is the VPS;
# ENVIRONMENT picks the bot (default staging).
set -euo pipefail

if [ $# -ne 1 ] || [ ! -f "$1" ]; then
    echo "usage: $0 <sakiot-speakers output>" >&2
    exit 2
fi
run_log="$1"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ -z "${SAKIOT_DEV_SSH:-}" ] && [ -f "$root/.env" ]; then
    SAKIOT_DEV_SSH="$(sed -n 's/^SAKIOT_DEV_SSH=//p' "$root/.env" | tail -n 1)"
fi
: "${SAKIOT_DEV_SSH:?set SAKIOT_DEV_SSH=user@vps-host}"
environment="${ENVIRONMENT:-staging}"
selector="job=\"otel-collector\",deployment_environment=\"$environment\""
control="$(mktemp -u "${TMPDIR:-/tmp}/sakiot-measure-XXXXXX")"
ssh_opts=(-o BatchMode=yes -o ControlMaster=auto -o "ControlPath=$control" -o ControlPersist=60)
trap 'ssh "${ssh_opts[@]}" -O exit "$SAKIOT_DEV_SSH" 2>/dev/null || true' EXIT

# The value of one PromQL expression at Unix time $2, or "nan".
query() {
    local expression="$1" at="$2"
    # shellcheck disable=SC2029 # expanded locally on purpose
    ssh "${ssh_opts[@]}" "$SAKIOT_DEV_SSH" \
        "curl -sfG http://localhost:9090/api/v1/query \
            --data-urlencode $(printf %q "query=$expression") \
            --data-urlencode time=$at" |
        jq -r '.data.result[0].value[1] // "nan"'
}

# Per-second rate of a counter over the window, summed over its series.
rate_of() {
    local metric="$1" extra="$2" seconds="$3" at="$4"
    query "sum(increase(${metric}{${selector}${extra}}[${seconds}s])) / ${seconds}" "$at"
}

# p99 of one voice tick stage over the window, in milliseconds.
tick_p99_ms() {
    local stage="$1" seconds="$2" at="$3"
    query "1000 * histogram_quantile(0.99, sum by (le) (increase(voice_tick_seconds_bucket{${selector},stage=\"${stage}\"}[${seconds}s])))" "$at"
}

printf '%s\n' "speakers	cpu_cores	tokio_busy_cores	packets_s	rtp_kbit_s	disk_write_kib_s	write_calls_s	rss_mb	recordings	tick_p99_receive_ms	tick_p99_record_ms"
grep '^window ' "$run_log" | while read -r _ speakers start end; do
    seconds=$((end - start))
    cpu="$(rate_of fbi_agent_process_cpu_seconds_total "" "$seconds" "$end")"
    tokio="$(rate_of tokio_runtime_busy_seconds_total "" "$seconds" "$end")"
    packets="$(rate_of audio_packets_received_total "" "$seconds" "$end")"
    rtp="$(rate_of voice_rtp_bytes_received_total "" "$seconds" "$end")"
    written="$(rate_of fbi_agent_process_io_bytes_total ',op="write"' "$seconds" "$end")"
    writes="$(rate_of fbi_agent_process_io_syscalls_total ',op="write"' "$seconds" "$end")"
    rss="$(query "avg(avg_over_time(process_rss_bytes{${selector}}[${seconds}s]))" "$end")"
    recordings="$(query "avg(avg_over_time(active_recordings{${selector}}[${seconds}s]))" "$end")"
    receive="$(tick_p99_ms receive "$seconds" "$end")"
    record="$(tick_p99_ms record "$seconds" "$end")"
    # A series Prometheus doesn't have prints as "-", not as zero.
    awk -v s="$speakers" -v c="$cpu" -v t="$tokio" -v p="$packets" -v r="$rtp" \
        -v w="$written" -v k="$writes" -v m="$rss" -v a="$recordings" \
        -v rv="$receive" -v rc="$record" '
        function show(value, scale, format) {
            if (value !~ /^-?[0-9.]+(e[-+]?[0-9]+)?$/) return "-"
            return sprintf(format, value * scale)
        }
        BEGIN {
            print s "\t" show(c, 1, "%.4f") "\t" show(t, 1, "%.4f") "\t" \
                show(p, 1, "%.1f") "\t" show(r, 8 / 1000, "%.1f") "\t" \
                show(w, 1 / 1024, "%.1f") "\t" show(k, 1, "%.1f") "\t" \
                show(m, 1 / 1e6, "%.1f") "\t" show(a, 1, "%.1f") "\t" \
                show(rv, 1, "%.3f") "\t" show(rc, 1, "%.3f")
        }'
done
