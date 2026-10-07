import type { components } from "../api/openapi";

type VoicePresence = components["schemas"]["VoicePresence"];
type PresenceChannel = components["schemas"]["PresenceChannel"];
type PresenceMember = components["schemas"]["PresenceMember"];
export type PresenceUpdate = components["schemas"]["PresenceUpdate"];

/** Discord ids exceed 2^53: compare them as integers, not as numbers. */
function compareIds(a: string, b: string): number {
	const left = BigInt(a);
	const right = BigInt(b);
	return left < right ? -1 : left > right ? 1 : 0;
}

function compareText(a: string, b: string): number {
	const left = a.toLowerCase();
	const right = b.toLowerCase();
	return left < right ? -1 : left > right ? 1 : 0;
}

/** The endpoint's order: channels by name, then id. */
function channelOrder(a: PresenceChannel, b: PresenceChannel): number {
	return compareText(a.name, b.name) || compareIds(a.channel_id, b.channel_id);
}

/** The endpoint's order: members by name, unnamed last, then id. */
function memberOrder(a: PresenceMember, b: PresenceMember): number {
	if (a.name != null && b.name != null) {
		const byName = compareText(a.name, b.name);
		if (byName !== 0) return byName;
	} else if (a.name != null || b.name != null) {
		return a.name == null ? 1 : -1;
	}
	return compareIds(a.user_id, b.user_id);
}

function insertSorted<T>(items: T[], item: T, order: (a: T, b: T) => number) {
	const at = items.findIndex((other) => order(item, other) < 0);
	items.splice(at < 0 ? items.length : at, 0, item);
}

/**
 * Applies pushed presence changes to a cached voice-presence list, in place
 * (an Immer draft). Each update says where one member is now: they leave
 * whatever channel they were in and, unless the update has no channel, join
 * theirs with their current state. Channels left empty disappear, as the
 * endpoint lists only occupied ones.
 */
export function applyPresenceUpdates(
	presence: VoicePresence,
	updates: PresenceUpdate[],
): void {
	for (const update of updates) {
		for (const channel of presence.channels) {
			const index = channel.members.findIndex(
				(member) => member.user_id === update.user_id,
			);
			if (index >= 0) channel.members.splice(index, 1);
		}
		const seat = update.channel;
		if (seat) {
			let channel = presence.channels.find(
				(c) => c.channel_id === seat.channel_id,
			);
			if (channel && channel.name !== seat.channel_name) {
				presence.channels.splice(presence.channels.indexOf(channel), 1);
				channel.name = seat.channel_name;
				insertSorted(presence.channels, channel, channelOrder);
			} else if (!channel) {
				channel = {
					channel_id: seat.channel_id,
					name: seat.channel_name,
					members: [],
				};
				insertSorted(presence.channels, channel, channelOrder);
			}
			insertSorted(channel.members, seat.member, memberOrder);
		}
	}
	for (let index = presence.channels.length - 1; index >= 0; index -= 1) {
		if (presence.channels[index]?.members.length === 0) {
			presence.channels.splice(index, 1);
		}
	}
}
