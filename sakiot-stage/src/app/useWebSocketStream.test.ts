import { describe, expect, test } from "bun:test";
import { shouldAttemptWebSocketRefresh } from "./useWebSocketStream";

describe("shouldAttemptWebSocketRefresh", () => {
	test("handles failed upgrades and explicit auth closes", () => {
		expect(shouldAttemptWebSocketRefresh(1006, false)).toBe(true);
		expect(shouldAttemptWebSocketRefresh(1008, false)).toBe(true);
		expect(shouldAttemptWebSocketRefresh(4001, false)).toBe(true);
	});

	test("does not rotate tokens repeatedly before a successful message", () => {
		expect(shouldAttemptWebSocketRefresh(1006, true)).toBe(false);
		expect(shouldAttemptWebSocketRefresh(1000, false)).toBe(false);
	});
});
