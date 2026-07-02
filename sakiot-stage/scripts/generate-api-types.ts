import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const openapiUrl =
	process.env.OPENAPI_URL ?? "http://localhost:8900/api-doc/openapi.json";
const outFile = resolve(process.env.OPENAPI_OUT ?? "src/api/openapi.ts");

await mkdir(dirname(outFile), { recursive: true });

const child = spawn("bunx", ["openapi-typescript", openapiUrl, "-o", outFile], {
	stdio: "inherit",
});

const code = await new Promise<number | null>((resolveCode) => {
	child.on("close", resolveCode);
});

if (code !== 0) {
	process.exit(code ?? 1);
}
