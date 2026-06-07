import { describe, expect, it } from "bun:test";
import { parseFileName } from "./parseFileName";

describe("parseFileName", () => {
	it("extracts timestamp, user id, and hyphenated username", () => {
		const ts = Date.UTC(2026, 0, 2, 3, 4, 5);
		const parsed = parseFileName(`${ts}-user-123-name-with-dashes.ogg`);
		const expected = new Date(ts);
		const pad = (n: number) => n.toString().padStart(2, "0");

		expect(parsed).toEqual({
			time: `${pad(expected.getHours())}:${pad(expected.getMinutes())}:${pad(expected.getSeconds())}`,
			userId: "user",
			username: "123-name-with-dashes",
		});
	});

	it("falls back to the base filename when timestamp is invalid", () => {
		expect(parseFileName("not-a-timestamp-user.ogg")).toEqual({
			time: "not-a-timestamp-user",
			userId: "a",
			username: "timestamp-user",
		});
	});
});
