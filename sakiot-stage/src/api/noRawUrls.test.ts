// biome-ignore-all lint/suspicious/noTemplateCurlyInString: this scanner and its fixtures match literal `${...}` text inside source snippets, so the pattern is data here rather than a string that forgot to be a template literal
import { describe, expect, it } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SOURCE_ROOT = join(fileURLToPath(new URL("..", import.meta.url)));

/**
 * Finds request URLs that were written by hand instead of built from
 * `API_ROUTES`. Those are the ones `tsc` cannot check against the backend
 * document, so they are exactly the ones that rot silently.
 *
 * A line may opt out with a `raw-url-ok` comment when the endpoint is not in
 * the OpenAPI document (for example the dev-login endpoint, which only exists
 * in dev-login builds).
 */
export function rawUrlOffenders(source: string): string[] {
	const offenders: string[] = [];
	const lines = source.split("\n");

	lines.forEach((line, index) => {
		if (line.includes("raw-url-ok")) return;

		const urlMatch = /url:\s*(["'`])([\s\S]*)$/.exec(line);
		if (urlMatch) {
			const [, quote, rest] = urlMatch;
			const builtByRegistry =
				rest.startsWith("apiUrl(") ||
				(quote === "`" && rest.startsWith("${apiUrl("));

			if (!builtByRegistry) {
				offenders.push(`${index + 1}: ${line.trim()}`);
			}
		}

		// Absolute media/HLS URLs go through `apiAbsoluteUrl` instead of
		// pasting BASE_API_URL into a template literal.
		if (line.includes("${BASE_API_URL}") && !line.includes("apiAbsoluteUrl(")) {
			offenders.push(`${index + 1}: ${line.trim()}`);
		}

		const fetchMatch = /fetchWithBQ\(\s*(["'`])([\s\S]*)$/.exec(line);
		if (fetchMatch && !fetchMatch[2].startsWith("apiUrl(")) {
			offenders.push(`${index + 1}: ${line.trim()}`);
		}

		// `authedFetch` takes the relative path directly, so a hand-written
		// template literal here bypasses the route registry just the same.
		const authedMatch = /authedFetch\(\s*(["'`])([\s\S]*)$/.exec(line);
		if (authedMatch && !authedMatch[2].startsWith("apiUrl(")) {
			offenders.push(`${index + 1}: ${line.trim()}`);
		}
		const declaresAuthedFetch =
			/\b(function|const|let)\s+authedFetch\s*\(/.test(line);
		if (!declaresAuthedFetch && /authedFetch\(\s*$/.test(line)) {
			const next = lines[index + 1]?.trim() ?? "";
			if (!next.includes("apiUrl(")) {
				offenders.push(`${index + 1}: ${line.trim()}`);
			}
		}
	});

	return offenders;
}

/**
 * Files exempt from the scan: the route registry itself and the fetch layer
 * that builds every request URL, both of which must reference BASE_API_URL
 * directly (and would otherwise import each other in a cycle).
 */
function isExempt(path: string): boolean {
	return (
		path.endsWith(`${sep}api${sep}routes.ts`) ||
		path.endsWith(`${sep}app${sep}authedFetch.ts`)
	);
}

function sourceFiles(directory: string): string[] {
	const files: string[] = [];
	for (const entry of readdirSync(directory, { withFileTypes: true })) {
		const path = join(directory, entry.name);
		if (entry.isDirectory()) {
			if (entry.name === "node_modules") continue;
			files.push(...sourceFiles(path));
			continue;
		}
		if (!/\.tsx?$/.test(entry.name)) continue;
		if (/\.test\.tsx?$/.test(entry.name)) continue;
		if (isExempt(path)) continue;
		files.push(path);
	}
	return files;
}

describe("api request URLs", () => {
	it("are all built from API_ROUTES", () => {
		const offenders = sourceFiles(SOURCE_ROOT).flatMap((file) =>
			rawUrlOffenders(readFileSync(file, "utf8")).map(
				(offender) => `${file.slice(SOURCE_ROOT.length)}:${offender}`,
			),
		);

		expect(offenders).toEqual([]);
	});

	it("flags a hand-written URL", () => {
		expect(rawUrlOffenders("url: `audio/clips/${guild_id}`")).toHaveLength(1);
		expect(rawUrlOffenders('url: "jamit"')).toHaveLength(1);
		expect(rawUrlOffenders('fetchWithBQ("users/current")')).toHaveLength(1);
		expect(rawUrlOffenders("url: apiUrl(API_ROUTES.clip)")).toEqual([]);
		expect(
			rawUrlOffenders("url: `${apiUrl(API_ROUTES.clips, { guild_id })}?x=1`"),
		).toEqual([]);
		expect(rawUrlOffenders("authedFetch(`audio/clips/${id}`)")).toHaveLength(1);
		expect(
			rawUrlOffenders("authedFetch(apiUrl(API_ROUTES.clip, params))"),
		).toEqual([]);
		expect(
			rawUrlOffenders("authedFetch(`dev_login?t=1`) // raw-url-ok"),
		).toEqual([]);
		expect(rawUrlOffenders("const u = `${BASE_API_URL}audio/x`;")).toHaveLength(
			1,
		);
		expect(
			rawUrlOffenders("const u = apiAbsoluteUrl(API_ROUTES.audio, p);"),
		).toEqual([]);
	});
});
