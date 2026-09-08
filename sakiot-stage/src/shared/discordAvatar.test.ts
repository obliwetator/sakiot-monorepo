import { describe, expect, it } from "bun:test";
import { discordAvatarUrl } from "./discordAvatar";

describe("discordAvatarUrl", () => {
	it("builds a CDN url from the stored hash", () => {
		expect(
			discordAvatarUrl({ user_id: "146638124288704513", avatar: "abc123" }),
		).toBe(
			"https://cdn.discordapp.com/avatars/146638124288704513/abc123.png?size=64",
		);
	});

	it("serves animated avatars as gif", () => {
		expect(discordAvatarUrl({ user_id: "1", avatar: "a_animated" })).toBe(
			"https://cdn.discordapp.com/avatars/1/a_animated.gif?size=64",
		);
	});

	it("falls back to the deterministic default avatar for an empty hash", () => {
		// 146638124288704513 >> 22 % 6 === 1
		expect(
			discordAvatarUrl({ user_id: "146638124288704513", avatar: "" }),
		).toBe("https://cdn.discordapp.com/embed/avatars/1.png");
	});

	it("returns null when there is no user", () => {
		expect(discordAvatarUrl(null)).toBeNull();
		expect(discordAvatarUrl(undefined)).toBeNull();
	});

	it("returns null for a non-snowflake id with no hash", () => {
		// Dev logins and e2e fixtures use ids like "current-user".
		expect(
			discordAvatarUrl({ user_id: "current-user", avatar: "" }),
		).toBeNull();
	});

	it("still builds a url for a non-snowflake id when a hash exists", () => {
		expect(discordAvatarUrl({ user_id: "current-user", avatar: "hash" })).toBe(
			"https://cdn.discordapp.com/avatars/current-user/hash.png?size=64",
		);
	});
});
