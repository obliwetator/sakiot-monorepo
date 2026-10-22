import { describe, expect, test } from "bun:test";
import type { UserGuilds } from "../Constants";
import { activePage, guildPages } from "./constants";

function guild(
	capabilities: UserGuilds["capabilities"],
	former = false,
): UserGuilds {
	return { id: "guild", name: "Guild", icon: null, capabilities, former };
}

describe("guildPages", () => {
	test("adds each admin page its capability opens", () => {
		expect(guildPages(guild([]))).toEqual([
			"Audio",
			"Clips",
			"Clip Editor",
			"Stamps",
		]);
		expect(guildPages(guild(["preview_roles"]))).toContain("Members");
		expect(guildPages(guild(["preview_roles"]))).not.toContain("Admin");
		expect(guildPages(guild(["manage_permissions"]))).toEqual([
			"Audio",
			"Clips",
			"Clip Editor",
			"Stamps",
			"Permissions",
		]);
	});

	test("offers a former member their own recordings, clips and stamps", () => {
		expect(guildPages(guild(["manage_settings"], true))).toEqual([
			"Audio",
			"Clips",
			"Stamps",
		]);
	});
});

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
		expect(activePage("/dashboard/123/admin/permissions")).toBe("Permissions");
		expect(activePage("/stamps/123")).toBe("Stamps");
		expect(activePage("/dashboard/123/missing")).toBeNull();
	});
});
