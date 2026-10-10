# sakiot-stage

The React 19 + Vite frontend. It talks only to `web-server`'s HTTP API and
realtime socket, through generated OpenAPI types. `cargo dev up` from the
repository root runs it with the API and database. From this directory:

```sh
bun install
bun run test          # unit tests
bun run test:e2e:ci   # Playwright, mocked API, desktop and mobile
bun run build         # Biome, type checks, bundle
```

The `dev`, `test`, `typecheck` and `build:bundle` scripts first compile
`sakiot-dsp` to WASM for the clip editor, which needs the pinned
`wasm-bindgen` CLI (see the root README). Builds require `VITE_API_URL`, from
the environment or the root Vite env files; there is no fallback origin.

Playwright needs Chromium (`bunx playwright install --with-deps chromium`) and
starts its own Vite server with mocked API routes. `test:e2e:ci` skips tests
tagged `@visual`; run `bunx playwright test` for visual review, following the
screenshot rules in [AGENTS.md](AGENTS.md).

## API types

`src/api/openapi.ts` is generated from `web-server`'s compile-time OpenAPI
document; neither command needs a running server or database:

```sh
bun run generate:api-types
bun run check:api-types   # fails when the checked-in file is stale
```

Set `OPENAPI_URL` only to generate from another source on purpose.

The generator lives in `scripts/codegen` with its own TypeScript 5, because
`openapi-typescript` emits through the TypeScript 5 compiler API
(`ts.factory`), which the TypeScript 7 compiler used here no longer exposes.
Both commands install it on first use. Once `openapi-typescript` supports
TypeScript 7 (`npm view openapi-typescript peerDependencies`), delete
`scripts/codegen`, move the package back into `devDependencies`, and call it
directly from `scripts/generate-api-types.ts`; `check:api-types` must stay
byte-identical.
