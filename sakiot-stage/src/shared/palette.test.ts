import { describe, expect, it } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { alpha, palette } from "./palette";

const SRC_ROOT = fileURLToPath(new URL("..", import.meta.url));
const STAGE_ROOT = fileURLToPath(new URL("../..", import.meta.url));

/** The only files allowed to declare colour values. */
const COLOUR_SOURCES = new Set(["index.css", "shared/palette.ts"]);

const COLOUR_PATTERNS = [
	/["'`]#[0-9a-fA-F]{3,8}["'`\s,)]/,
	/["'`]rgba?\([^"'`]*\)["'`]/,
	/["'`](?:oklch|oklab|hsla?)\([^"'`]*\)["'`]/,
];

/** Line-numbered colour literals in a source string. */
export function colourLiterals(source: string): string[] {
	return source
		.split("\n")
		.flatMap((line, index) =>
			COLOUR_PATTERNS.some((pattern) => pattern.test(line))
				? [`${index + 1}: ${line.trim()}`]
				: [],
		);
}

describe("colour literals", () => {
	it("are declared only in index.css and shared/palette.ts", () => {
		const offenders: string[] = [];

		for (const entry of readdirSync(SRC_ROOT, { recursive: true })) {
			if (!/\.(ts|tsx|css)$/.test(entry)) continue;
			// Tests assert literal values on purpose, so they are out of scope.
			if (entry.endsWith(".test.ts") || entry.endsWith(".test.tsx")) continue;
			if (COLOUR_SOURCES.has(entry)) continue;

			for (const hit of colourLiterals(
				readFileSync(join(SRC_ROOT, entry), "utf8"),
			)) {
				offenders.push(`${entry}:${hit}`);
			}
		}

		expect(offenders).toEqual([]);
	});

	it("detector flags a hand-written colour and passes palette references", () => {
		expect(colourLiterals('color: "#ff00ff";')).toHaveLength(1);
		expect(colourLiterals('stroke="rgba(125, 211, 252, 0.9)"')).toHaveLength(1);
		expect(colourLiterals("color: palette.sky500;")).toEqual([]);
		expect(colourLiterals("color: alpha(palette.sky300, 0.9);")).toEqual([]);
	});

	it("alpha() renders the same rgba() strings the palette used to inline", () => {
		expect(alpha(palette.slate400, 0.72)).toBe("rgba(148, 163, 184, 0.72)");
		expect(alpha(palette.white, 0.5)).toBe("rgba(255, 255, 255, 0.5)");
		expect(alpha(palette.slate900, 0.5)).toBe("rgba(15, 23, 42, 0.5)");
	});

	it("stays aligned with the theme token it duplicates", () => {
		const css = readFileSync(join(STAGE_ROOT, "src/index.css"), "utf8");
		const muted = /--color-muted:\s*([^;]+);/.exec(css)?.[1]?.trim();

		expect(palette.slate400).toBe(muted);
	});
});
