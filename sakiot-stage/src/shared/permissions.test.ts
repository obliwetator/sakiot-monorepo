import { describe, expect, it } from "bun:test";
import type { UserGuilds } from "../Constants";
import { canDeleteClip, holds } from "./permissions";

function guild(capabilities: UserGuilds["capabilities"]): UserGuilds {
	return {
		id: "guild",
		name: "Guild",
		icon: null,
		capabilities,
		former: false,
	};
}

describe("holds", () => {
	it("reads the guild's capabilities", () => {
		expect(holds(guild(["manage_settings"]), "manage_settings")).toBe(true);
		expect(holds(guild(["manage_settings"]), "preview_roles")).toBe(false);
	});

	it("holds nothing without a guild", () => {
		expect(holds(null, "manage_settings")).toBe(false);
		expect(holds(undefined, "delete_recordings")).toBe(false);
	});
});

describe("canDeleteClip", () => {
	it("allows clip owners", () => {
		expect(canDeleteClip(guild([]), "user-1", "user-1")).toBe(true);
	});

	it("allows clip moderators", () => {
		expect(canDeleteClip(guild(["moderate_clips"]), "user-1", "user-2")).toBe(
			true,
		);
	});

	it("rejects anyone else", () => {
		expect(canDeleteClip(guild(["manage_settings"]), "user-1", "user-2")).toBe(
			false,
		);
		expect(canDeleteClip(guild([]), null, "user-2")).toBe(false);
	});
});
