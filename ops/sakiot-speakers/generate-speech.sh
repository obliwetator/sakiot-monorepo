#!/usr/bin/env bash
# Regenerates speech.ogg, the speakers' audio: 30 s of band-limited pink noise
# (the 120–3400 Hz band of web-server/src/loadtest/media.rs) as mono Opus in
# 20 ms frames. 66 kbit/s VOIP is the highest rate at which libopus keeps
# coding it in Hybrid full band, as Discord clients code voice: real client
# packets averaged 163 B, these 156 B. The noise is continuous; the speakers
# pause it to go quiet, as a client stops sending when nobody talks.
#
# Committed rather than generated per run, so every run sends the same
# packets whatever ffmpeg version is installed.
set -euo pipefail
out="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/speech.ogg"
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "anoisesrc=color=pink:amplitude=0.3:seed=7:sample_rate=48000,lowpass=f=3400,highpass=f=120" \
    -t 30 -ac 1 -ar 48000 \
    -c:a libopus -b:a 66k -vbr on -frame_duration 20 -application voip \
    -page_duration 500000 -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
    "$out"
