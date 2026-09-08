import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const API_SLICE_PATH = fileURLToPath(
	new URL("../app/apiSlice.ts", import.meta.url),
);

/**
 * Finds request URLs that were written by hand instead of built from
 * `API_ROUTES`. Those are the ones `tsc` cannot check against the backend
 * document, so they are exactly the ones that rot silently.
 */
export function rawUrlOffenders(source: string): string[] {
	const offenders: string[] = [];

	source.split("\n").forEach((line, index) => {
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

		const fetchMatch = /fetchWithBQ\(\s*(["'`])([\s\S]*)$/.exec(line);
		if (fetchMatch && !fetchMatch[2].startsWith("apiUrl(")) {
			offenders.push(`${index + 1}: ${line.trim()}`);
		}
	});

	return offenders;
}

describe("apiSlice request URLs", () => {
	it("are all built from API_ROUTES", () => {
		const source = readFileSync(API_SLICE_PATH, "utf8");

		expect(rawUrlOffenders(source)).toEqual([]);
	});

	it("flags a hand-written URL", () => {
		expect(rawUrlOffenders("url: `audio/clips/${guild_id}`")).toHaveLength(1);
		expect(rawUrlOffenders('url: "jamit"')).toHaveLength(1);
		expect(rawUrlOffenders('fetchWithBQ("users/current")')).toHaveLength(1);
		expect(rawUrlOffenders("url: apiUrl(API_ROUTES.clip)")).toEqual([]);
		expect(
			rawUrlOffenders("url: `${apiUrl(API_ROUTES.clips, { guild_id })}?x=1`"),
		).toEqual([]);
	});
});
