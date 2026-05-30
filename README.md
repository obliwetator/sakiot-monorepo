# Sakiot Paths

Sakiot Paths is the small shared Rust crate that defines the canonical on-disk
layout and URL conventions for Sakiot voice recordings, no-silence recordings,
waveform data, live HLS cache data, and clips. Runtime media defaults to the
shared `../data` directory and can be overridden with `SAKIOT_DATA_DIR`, which
is the intended container mount point knob.

This project is functional, but it is not packaged as a supported application.
No support is provided for running, deploying, configuring, or operating it. For
now, you have to figure that out yourself from the code and from how the sibling
projects use it.

Local development defaults to `../data`. Container deployments should set
`SAKIOT_DATA_DIR=/data` and mount the shared media volume there.

## Role In The System

Sakiot Paths is linked with the other projects in this directory to make the
whole Sakiot application:

- `FBI-agent` uses it when writing recordings.
- `web_server` uses it when reading recordings, serving media, and building
  media URLs.
- `sakiot_stage` depends on the backend behavior that comes from these shared
  conventions.

## What It Does

- Defines canonical recording roots and filename conventions.
- Builds recording, no-silence, waveform, live playlist, and live segment paths.
- Builds API URL paths for recordings, waveforms, and live session playback.
- Keeps writer and reader projects aligned around one filesystem layout.

## Status

This is a utility crate for the local Sakiot stack, not a standalone product. It
is useful because the surrounding projects agree to use the same layout.
