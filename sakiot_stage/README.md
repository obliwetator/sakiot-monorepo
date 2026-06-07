# Sakiot Stage

Sakiot Stage is the React/Vite frontend for the Sakiot system. It provides the
browser UI for logging in, browsing Discord guild-related content, viewing and
playing recordings, working with live audio state, clips, stamps, waveforms, and
admin-facing controls exposed by the backend.

This project is functional, but it is not packaged as a supported application.
No support is provided for running, deploying, configuring, or operating it. For
now, you have to figure that out yourself from the code, scripts, generated API
types, and local setup.

## Role In The System

Sakiot Stage is linked with the other projects in this directory to make the
whole Sakiot application:

- `web_server` provides the authenticated HTTP API and media endpoints this UI
  consumes.
- `FBI-agent` records the Discord voice data that eventually appears in the UI.
- `sakiot-paths` defines shared path conventions used by the backend pieces that
  serve the data shown here.

## What It Does

- Runs a React 19 application through Vite.
- Uses Material UI for application shell and interface components.
- Talks to the backend through generated OpenAPI types and authenticated fetch
  helpers.
- Provides protected routes behind the login/bootstrap flow.
- Displays recording, live audio, waveform, clip, stamp, and guild-related
  workflows.
- Includes tests for selected shared utilities and auth fetch behavior.

## Status

This is personal/project code, not a turnkey product. It expects the matching
backend, generated API types, auth configuration, and deployment environment to
already make sense.
