import { describe, expect, test } from "bun:test";
import type { components } from "../api/openapi";
import { applyPresenceUpdates, type PresenceUpdate } from "./patchPresence";

type VoicePresence = components["schemas"]["VoicePresence"];

const member = (user_id: string, name: string | null, self_mute = false) => ({
	user_id,
	name,
	is_bot: false,
	self_mute,
	self_deaf: false,
	server_mute: false,
	server_deaf: false,
	streaming: false,
	video: false,
});

const seat = (
	user_id: string,
	name: string | null,
	channel_id: string,
	channel_name: string,
	self_mute = false,
): PresenceUpdate => ({
	user_id,
	channel: {
		channel_id,
		channel_name,
		member: member(user_id, name, self_mute),
	},
});

function presence(): VoicePresence {
	return {
		available: true,
		channels: [
			{
				channel_id: "100",
				name: "General",
				members: [member("1", "Alice"), member("3", "Carol")],
			},
			{
				channel_id: "9007199254740993",
				name: "Music",
				members: [member("4", "Dan")],
			},
		],
	};
}

const names = (list: VoicePresence) =>
	list.channels.map((channel) => [
		channel.name,
		channel.members.map((m) => m.name ?? m.user_id),
	]);

describe("applyPresenceUpdates", () => {
	test("a member joining takes their place by name", () => {
		const list = presence();
		applyPresenceUpdates(list, [seat("2", "bob", "100", "General")]);
		expect(names(list)).toEqual([
			["General", ["Alice", "bob", "Carol"]],
			["Music", ["Dan"]],
		]);
	});

	test("unnamed members go last, by id", () => {
		const list = presence();
		applyPresenceUpdates(list, [
			seat("20", null, "100", "General"),
			seat("10", null, "100", "General"),
		]);
		expect(names(list)[0]).toEqual(["General", ["Alice", "Carol", "10", "20"]]);
	});

	test("a move leaves the old channel and drops it once empty", () => {
		const list = presence();
		applyPresenceUpdates(list, [seat("4", "Dan", "100", "General")]);
		expect(names(list)).toEqual([["General", ["Alice", "Carol", "Dan"]]]);
	});

	test("a state change replaces the member in place", () => {
		const list = presence();
		applyPresenceUpdates(list, [seat("1", "Alice", "100", "General", true)]);
		expect(list.channels[0]?.members[0]).toEqual(member("1", "Alice", true));
		expect(list.channels[0]?.members).toHaveLength(2);
	});

	test("a member leaving is removed", () => {
		const list = presence();
		applyPresenceUpdates(list, [{ user_id: "3" }, { user_id: "4" }]);
		expect(names(list)).toEqual([["General", ["Alice"]]]);
	});

	test("a new channel is placed by name, then id", () => {
		const list = presence();
		applyPresenceUpdates(list, [
			seat("5", "Eve", "9007199254740995", "music"),
			seat("6", "Finn", "50", "Alpha"),
		]);
		expect(list.channels.map((c) => c.channel_id)).toEqual([
			"50",
			"100",
			"9007199254740993",
			"9007199254740995",
		]);
	});

	test("a renamed channel moves to its new place", () => {
		const list = presence();
		applyPresenceUpdates(list, [
			seat("4", "Dan", "9007199254740993", "Arcade"),
		]);
		expect(names(list)).toEqual([
			["Arcade", ["Dan"]],
			["General", ["Alice", "Carol"]],
		]);
	});
});
