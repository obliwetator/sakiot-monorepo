import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const dspRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const mappings = [
	["/tone/", resolve(dspRoot, "node_modules/tone/build/esm")],
	[
		"/automation/",
		resolve(dspRoot, "node_modules/automation-events/build/es2019"),
	],
	[
		"/standardized/",
		resolve(dspRoot, "node_modules/standardized-audio-context/build/es2019"),
	],
	["/tslib/", resolve(dspRoot, "node_modules/tslib")],
	["/pkg/", resolve(dspRoot, "pkg")],
	["/harness.js", resolve(dspRoot, "browser/harness.js")],
	["/passthrough-worklet.js", resolve(dspRoot, "web/passthrough-worklet.js")],
	["/worklet.js", resolve(dspRoot, "pkg/sakiot-dsp-worklet.bundle.js")],
	["/", resolve(dspRoot, "browser/index.html")],
];

const mimeTypes = {
	".html": "text/html; charset=utf-8",
	".js": "text/javascript; charset=utf-8",
	".mjs": "text/javascript; charset=utf-8",
	".wasm": "application/wasm",
};

function mappedPath(pathname) {
	for (const [prefix, target] of mappings) {
		if (prefix.endsWith("/") && prefix !== "/" && pathname.startsWith(prefix)) {
			const candidate = resolve(target, pathname.slice(prefix.length));
			if (candidate === target || candidate.startsWith(`${target}${sep}`)) return candidate;
		} else if (pathname === prefix) {
			return target;
		}
	}
	return null;
}

async function existingModulePath(path) {
	if (!path) return null;
	for (const candidate of extname(path) ? [path] : [path, `${path}.js`]) {
		try {
			if ((await stat(candidate)).isFile()) return candidate;
		} catch (error) {
			if (error?.code !== "ENOENT") throw error;
		}
	}
	return null;
}

export async function startHarnessServer() {
	const server = createServer(async (request, response) => {
		try {
			const pathname = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
			const path = await existingModulePath(mappedPath(pathname));
			if (!path) {
				response.writeHead(404).end("Not found");
				return;
			}
			response.writeHead(200, {
				"Content-Type": mimeTypes[extname(path)] ?? "application/octet-stream",
				"Cache-Control": "no-store",
			});
			createReadStream(path).pipe(response);
		} catch (error) {
			response.writeHead(500).end(String(error));
		}
	});
	await new Promise((resolveListen, reject) => {
		server.once("error", reject);
		server.listen(0, "127.0.0.1", resolveListen);
	});
	const address = server.address();
	if (!address || typeof address === "string") throw new Error("No harness address");
	return {
		url: `http://127.0.0.1:${address.port}`,
		close: () => new Promise((resolveClose) => server.close(resolveClose)),
	};
}
