import { describe, expect, test } from "bun:test";
import { visiblePlaybackTab } from "./playbackTabs";

describe("visiblePlaybackTab", () => {
	test("shows the controller's timeline until the mix is chosen", () => {
		expect(visiblePlaybackTab(false, true, "normal")).toBe("normal");
		expect(visiblePlaybackTab(false, true, "silence")).toBe("silence");
		expect(visiblePlaybackTab(true, true, "silence")).toBe("mix");
	});

	test("keeps the mix on screen when the controller switches timelines", () => {
		expect(visiblePlaybackTab(true, true, "normal")).toBe("mix");
		expect(visiblePlaybackTab(true, true, "silence")).toBe("mix");
	});

	test("falls back to the active timeline when the mix loses its tracks", () => {
		expect(visiblePlaybackTab(true, false, "normal")).toBe("normal");
		expect(visiblePlaybackTab(true, false, "silence")).toBe("silence");
	});
});
