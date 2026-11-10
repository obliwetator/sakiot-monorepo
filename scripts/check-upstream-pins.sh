#!/usr/bin/env bash
# Fails when a crate that holds back a vulnerable dependency has a release
# other than the one last reviewed. deny.toml's [advisories] ignore list keeps
# an entry only while one of these crates pins the old code, so a release is
# the moment to try the upgrade. The Dependabot alerts for the same advisories
# are dismissed, so this is the reminder. Run weekly by
# .github/workflows/dependency-advisories.yml.
#
# After a release: try the upgrade, update or delete the deny.toml entries it
# affects, then record the version you reviewed in the check below.
set -euo pipefail

USER_AGENT="sakiot-monorepo upstream pin check (https://github.com/obliwetator/sakiot-monorepo)"
stale=0

# check <crate> <version last reviewed> <what it holds back>
check() {
    local crate=$1 reviewed=$2 holds=$3 latest
    latest=$(curl -fsS --retry 3 --user-agent "$USER_AGENT" \
        "https://crates.io/api/v1/crates/$crate" | jq -r '.crate.max_stable_version')
    if [ -z "$latest" ] || [ "$latest" = null ]; then
        echo "::error::could not read the latest $crate release from crates.io"
        stale=1
    elif [ "$latest" = "$reviewed" ]; then
        echo "$crate $latest: as reviewed"
    else
        echo "::error::crates.io's latest $crate is $latest, last reviewed $reviewed. It $holds. Try the upgrade, update deny.toml's ignore list, then record $latest in $0."
        stale=1
    fi
}

check serenity 0.12.5 "pins rustls 0.22 and rustls-webpki 0.102 through tokio-tungstenite 0.21"
check songbird 0.6.1 "pins davey 0.1, and through it libcrux and crypto-bigint 0.5, and ringbuf 0.4"

exit "$stale"
