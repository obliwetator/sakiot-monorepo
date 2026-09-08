import { describe, expect, it } from "bun:test";
import { API_ROUTES, apiUrl } from "./routes";

const SAMPLE: Record<string, string> = {
	guild_id: "1",
	channel_id: "2",
	year: "2026",
	month: "7",
	file_name: "clip.ogg",
	clip_id: "abc",
	recording_session_id: "9",
	role_id: "77",
	user_id: "5",
	stem: "stem",
	file: "file.ogg",
};

describe("apiUrl", () => {
	it("resolves every documented route with no placeholder left behind", () => {
		for (const route of Object.values(API_ROUTES)) {
			const url = apiUrl(route, SAMPLE);

			expect(url).not.toContain("{");
			// VITE_API_URL already ends in /api/, so the prefix is stripped and
			// fetchBaseQuery resolves the rest against it.
			expect(url.startsWith("/")).toBe(false);
			expect(url.length).toBeGreaterThan(0);
		}
	});

	it("percent-encodes parameter values", () => {
		expect(apiUrl(API_ROUTES.clip, { guild_id: "1", clip_id: "a/b c" })).toBe(
			"audio/clips/1/a%2Fb%20c",
		);
	});

	it("throws when a placeholder has no value", () => {
		expect(() => apiUrl(API_ROUTES.clip, { guild_id: "1" })).toThrow(
			/missing value for \{clip_id\}/,
		);
	});
});
