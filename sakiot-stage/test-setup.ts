// bun test has no Vite to bake VITE_* config into import.meta.env, so mirror
// the required build-time variable here. `??=` keeps a real value when the
// caller (e.g. the deploy engine's env file) already provides one.
process.env.VITE_API_URL ??= "https://sakiot.test/api/";
