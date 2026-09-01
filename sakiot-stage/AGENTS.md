# Frontend agent guidance

## Visual verification

- Use Playwright as the canonical browser harness for frontend behavior and visual checks.
- Run commands from `sakiot-stage/`.
- Use `bunx playwright test` for the complete desktop/mobile suite. Narrow by file or `--grep` while investigating, then rerun the relevant complete suite.
- Tests must use mocked/local APIs. Never point Playwright at production services or mutate production data.
- Exercise both `desktop-chromium` and `mobile-chromium` unless the behavior is intentionally viewport-specific.
- CI runs `bun run test:e2e:ci`, which excludes tests tagged `@visual`. Run visual tests manually only when visual review is requested.
- On a screenshot failure, inspect the expected, actual, and diff PNGs with a vision-capable image tool. Inspect the Playwright trace when interaction or timing may explain the result.
- Treat screenshot drift as a possible product regression. Do not run `--update-snapshots` merely to make a test pass.
- Update committed screenshot baselines only after the changed rendering has been reviewed and explicitly accepted.
- Treat hard-coded color, spacing, and geometry assertions as product requirements until proven stale. Diagnose the component and migration diff before changing the assertion.
- Prefer semantic assertions such as role, accessible state, visibility, focus, and interaction outcome. Use exact pixels or computed colors only when appearance itself is the requirement.
- Report viewport, failing assertion, expected/actual difference, artifact paths, and whether the evidence indicates a regression, stale baseline, or environment issue.
- Preserve `test-results/` and traces long enough for review. They are generated artifacts and must not be committed.

## Common commands

```bash
bun run test:e2e:ci
bunx playwright test
bunx playwright test e2e/admin-cooldowns.spec.ts --grep "visual baseline"
bunx playwright show-trace test-results/<test-name>/trace.zip
```

If Chromium is absent, report the environment gap. Installation command:

```bash
bunx playwright install --with-deps chromium
```
