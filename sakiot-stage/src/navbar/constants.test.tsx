import { describe, expect, test } from "bun:test";
import { activePage } from "./constants";

describe("activePage", () => {
	test("marks each media route and its nested pages", () => {
		expect(activePage("/dashboard/123/audio/session/5")).toBe("Audio");
		expect(activePage("/dashboard/123/clips/a-clip")).toBe("Clips");
		expect(activePage("/dashboard/123/clips/editor")).toBe("Clip Editor");
	});

	test("distinguishes admin sections and leaves unknown paths unmarked", () => {
		expect(activePage("/dashboard/123/admin/cooldowns")).toBe("Admin");
		expect(activePage("/dashboard/123/admin/voice-settings")).toBe(
			"Voice Settings",
		);
		expect(activePage("/dashboard/123/members")).toBe("Members");
		expect(activePage("/stamps/123")).toBe("Stamps");
		expect(activePage("/dashboard/123/missing")).toBeNull();
	});
});
