import { describe, expect, it } from "bun:test";
import type { UserGuilds } from "../Constants";
import { canDeleteClip, isGuildAdmin } from "./permissions";

function guild(overrides: Partial<UserGuilds>): UserGuilds {
	return {
		id: "guild",
		name: "Guild",
		icon: null,
		owner: false,
		permissions: "0",
		...overrides,
	} as UserGuilds;
}

describe("isGuildAdmin", () => {
	it("accepts owners", () => {
		expect(isGuildAdmin(guild({ owner: true }))).toBe(true);
	});

	it("accepts administrator and manage guild permission bits", () => {
		expect(isGuildAdmin(guild({ permissions: "8" }))).toBe(true);
		expect(isGuildAdmin(guild({ permissions: "32" }))).toBe(true);
	});

	it("rejects missing, invalid, or unrelated permissions", () => {
		expect(isGuildAdmin(null)).toBe(false);
		expect(isGuildAdmin(guild({ permissions: "16" }))).toBe(false);
		expect(isGuildAdmin(guild({ permissions: "not-a-number" }))).toBe(false);
	});
});

describe("canDeleteClip", () => {
	it("allows clip owners", () => {
		expect(canDeleteClip(guild({}), "user-1", "user-1")).toBe(true);
	});

	it("allows guild managers", () => {
		expect(
			canDeleteClip(guild({ permissions: "32" }), "user-1", "user-2"),
		).toBe(true);
	});

	it("rejects non-owner non-manager users", () => {
		expect(canDeleteClip(guild({}), "user-1", "user-2")).toBe(false);
		expect(canDeleteClip(guild({}), null, "user-2")).toBe(false);
	});
});
