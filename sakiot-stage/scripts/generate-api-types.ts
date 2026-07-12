import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const frontendRoot = resolve(scriptDirectory, "..");
const repositoryRoot = resolve(frontendRoot, "..");
const outputFile = resolve(
	process.env.OPENAPI_OUT ?? resolve(frontendRoot, "src/api/openapi.ts"),
);
const checkOnly = process.argv.slice(2).includes("--check");

function waitForExit(
	child: ReturnType<typeof spawn>,
	label: string,
): Promise<void> {
	return new Promise((resolvePromise, reject) => {
		child.once("error", reject);
		child.once("close", (code, signal) => {
			if (code === 0) {
				resolvePromise();
				return;
			}

			reject(
				new Error(
					`${label} failed (${signal ? `signal ${signal}` : `exit ${code ?? "unknown"}`})`,
				),
			);
		});
	});
}

async function exportOpenApi(destination: string): Promise<void> {
	const child = spawn(
		"cargo",
		[
			"run",
			"--locked",
			"--quiet",
			"-p",
			"web_server",
			"--bin",
			"export_openapi",
		],
		{
			cwd: repositoryRoot,
			env: { ...process.env, SQLX_OFFLINE: "true" },
			stdio: ["ignore", "pipe", "inherit"],
		},
	);
	const chunks: Buffer[] = [];
	child.stdout.on("data", (chunk: Buffer) => chunks.push(chunk));

	await waitForExit(child, "OpenAPI export");
	await writeFile(destination, Buffer.concat(chunks));
}

async function generateTypes(
	source: string,
	destination: string,
): Promise<void> {
	const child = spawn(
		"bunx",
		["openapi-typescript", source, "-o", destination],
		{
			cwd: frontendRoot,
			stdio: "inherit",
		},
	);

	await waitForExit(child, "OpenAPI type generation");
}

async function formatTypes(destination: string): Promise<void> {
	const child = spawn("bunx", ["biome", "format", "--write", destination], {
		cwd: frontendRoot,
		stdio: "inherit",
	});

	await waitForExit(child, "Generated OpenAPI type formatting");
}

async function main(): Promise<void> {
	const temporaryDirectory = await mkdtemp(join(tmpdir(), "sakiot-openapi-"));

	try {
		const generatedFile = join(temporaryDirectory, "openapi.ts");
		let source = process.env.OPENAPI_URL;

		if (!source) {
			source = join(temporaryDirectory, "openapi.json");
			await exportOpenApi(source);
		}

		await generateTypes(source, generatedFile);
		await formatTypes(generatedFile);
		const generated = await readFile(generatedFile);

		if (checkOnly) {
			let checkedIn: Buffer | undefined;
			try {
				checkedIn = await readFile(outputFile);
			} catch (error) {
				if (
					!(error instanceof Error) ||
					!("code" in error) ||
					error.code !== "ENOENT"
				) {
					throw error;
				}
			}

			if (!checkedIn?.equals(generated)) {
				process.stderr.write(
					"Generated OpenAPI types are stale. Run `bun run generate:api-types` from sakiot-stage.\n",
				);
				process.exitCode = 1;
			}
			return;
		}

		await mkdir(dirname(outputFile), { recursive: true });
		await writeFile(outputFile, generated);
		process.stdout.write(`Wrote ${outputFile}\n`);
	} finally {
		await rm(temporaryDirectory, { recursive: true, force: true });
	}
}

try {
	await main();
} catch (error) {
	process.stderr.write(
		`API type generation failed: ${error instanceof Error ? error.message : String(error)}\n`,
	);
	process.exitCode = 1;
}
