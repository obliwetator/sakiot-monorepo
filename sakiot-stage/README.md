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

- `web-server` provides the authenticated HTTP API and media endpoints this UI
  consumes.
- `fbi-agent` records the Discord voice data that eventually appears in the UI.
- `sakiot-paths` defines shared path conventions used by the backend pieces that
  serve the data shown here.

## What It Does

- Runs a React 19 application through Vite.
- Uses React Aria wrappers and Tailwind for the application shell and interface components.
- Talks to the backend through generated OpenAPI types and authenticated fetch
  helpers.
- Provides protected routes behind the login/bootstrap flow.
- Displays recording, live audio, waveform, clip, stamp, and guild-related
  workflows.
- Includes tests for selected shared utilities and auth fetch behavior.

## API Types

The checked-in API types come from `web-server`'s compile-time OpenAPI document.
Neither command needs a running server or database:

```sh
bun run generate:api-types
bun run check:api-types
```

The check command generates into a temporary file and fails when
`src/api/openapi.ts` is stale. Set `OPENAPI_URL` only to use another OpenAPI
source intentionally.

The generator itself lives in `scripts/codegen`, which pins its own TypeScript 5
alongside `openapi-typescript`. `openapi-typescript` emits its output through
the TypeScript 5 compiler API (`ts.factory`), which the native TypeScript 7
compiler used by the frontend no longer exposes. Both commands install that
toolchain on first use, so no extra setup step is required.

This split is temporary. Check upstream periodically:

```sh
npm view openapi-typescript peerDependencies
```

As of 2026-07-25 the newest release is 7.13.0 and still requires
`typescript: ^5.x`. Once the peer range admits TypeScript 7, delete
`scripts/codegen`, move `openapi-typescript` back into `devDependencies`, and
revert `scripts/generate-api-types.ts` to invoking it directly.
`bun run check:api-types` must stay byte-identical against `src/api/openapi.ts`.

## Status

This is personal/project code, not a turnkey product. It expects the matching
backend, generated API types, auth configuration, and deployment environment to
already make sense.
