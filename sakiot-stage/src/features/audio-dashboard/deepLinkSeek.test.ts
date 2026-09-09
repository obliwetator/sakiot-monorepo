import { describe, expect, test } from "bun:test";
import { deepLinkSeekSeconds } from "./deepLinkSeek";

describe("deepLinkSeekSeconds", () => {
	test("returns the requested seconds", () => {
		expect(deepLinkSeekSeconds("?t=12.5")).toBe(12.5);
		expect(deepLinkSeekSeconds("?t=0")).toBe(0);
		expect(deepLinkSeekSeconds("?t=-3")).toBe(-3);
		expect(deepLinkSeekSeconds("?foo=1&t=90&bar=2")).toBe(90);
	});

	test("returns null when the parameter is absent or blank", () => {
		expect(deepLinkSeekSeconds("")).toBeNull();
		expect(deepLinkSeekSeconds("?foo=1")).toBeNull();
		expect(deepLinkSeekSeconds("?t=")).toBeNull();
		expect(deepLinkSeekSeconds("?t=%20")).toBeNull();
	});

	test("returns null for values that would throw when assigned", () => {
		// Assigning NaN or Infinity to currentTime throws a TypeError, which
		// would abort the canplay handler and leave the player unusable.
		for (const value of ["abc", "NaN", "Infinity", "-Infinity", "%20"]) {
			expect(deepLinkSeekSeconds(`?t=${value}`)).toBeNull();
		}
	});

	test("keeps parseFloat's lenient prefix parsing", () => {
		// Same behaviour as the parseFloat(t) this replaced: a numeric prefix
		// still seeks, trailing garbage is ignored.
		expect(deepLinkSeekSeconds("?t=12abc")).toBe(12);
	});
});
