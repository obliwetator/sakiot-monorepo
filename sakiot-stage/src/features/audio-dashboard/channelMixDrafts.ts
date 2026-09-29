import type {
	ChannelMixParticipantSettings,
	ChannelMixTrack,
} from "../../app/apiSlice";
import { clampChannelMixGain } from "./channelMixState";

const STORAGE_KEY = "sakiot.channel-mix.drafts.v1";

type DraftStore = Record<string, ChannelMixParticipantSettings[]>;

/** A stored participant's settings, or null when the entry is not one. */
function storedParticipant(
	value: unknown,
): ChannelMixParticipantSettings | null {
	if (typeof value !== "object" || value === null) return null;
	if (!("user_id" in value) || typeof value.user_id !== "string") return null;
	return {
		user_id: value.user_id,
		gain_db:
			"gain_db" in value && typeof value.gain_db === "number"
				? value.gain_db
				: 0,
		muted: "muted" in value && value.muted === true,
	};
}

/**
 * Storage outlives the code that wrote it and can be edited by hand, so every
 * entry is validated; anything that is not a participant list is dropped.
 */
function readStore(): DraftStore {
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return {};
		const parsed: unknown = JSON.parse(raw);
		if (typeof parsed !== "object" || parsed === null) return {};
		if (Array.isArray(parsed)) return {};
		const store: DraftStore = {};
		for (const [sessionId, settings] of Object.entries(parsed)) {
			if (!Array.isArray(settings)) continue;
			store[sessionId] = settings.flatMap(
				(value: unknown) => storedParticipant(value) ?? [],
			);
		}
		return store;
	} catch {
		return {};
	}
}

function writeStore(store: DraftStore): void {
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
	} catch {
		// Storage is optional; keep the in-memory draft usable.
	}
}

function cleanSettings(
	settings: readonly ChannelMixParticipantSettings[],
): ChannelMixParticipantSettings[] {
	return settings
		.filter((participant) => typeof participant.user_id === "string")
		.map((participant) => ({
			user_id: participant.user_id,
			gain_db: clampChannelMixGain(participant.gain_db),
			muted: Boolean(participant.muted),
		}))
		.sort((left, right) => left.user_id.localeCompare(right.user_id));
}

export function readChannelMixDraft(
	sessionId: string,
): ChannelMixParticipantSettings[] {
	return cleanSettings(readStore()[sessionId] ?? []);
}

export function writeChannelMixDraft(
	sessionId: string,
	settings: readonly ChannelMixParticipantSettings[],
): void {
	const store = readStore();
	store[sessionId] = cleanSettings(settings);
	writeStore(store);
}

/** Local draft wins, then the server's last valid render, then defaults. */
export function mergeChannelMixDraft(
	sessionId: string,
	tracks: readonly ChannelMixTrack[],
	serverSettings: readonly ChannelMixParticipantSettings[] | undefined,
	current: readonly ChannelMixParticipantSettings[] = [],
): ChannelMixParticipantSettings[] {
	const local = new Map(
		readChannelMixDraft(sessionId).map((item) => [item.user_id, item]),
	);
	const server = new Map(
		cleanSettings(serverSettings ?? []).map((item) => [item.user_id, item]),
	);
	const inMemory = new Map(
		cleanSettings(current).map((item) => [item.user_id, item]),
	);
	return tracks
		.map(
			(track) =>
				local.get(track.user_id) ??
				inMemory.get(track.user_id) ??
				server.get(track.user_id) ?? {
					user_id: track.user_id,
					gain_db: 0,
					muted: false,
				},
		)
		.map((item) => ({ ...item }))
		.sort((left, right) => left.user_id.localeCompare(right.user_id));
}

export function channelMixRenderSettingsEqual(
	left: readonly ChannelMixParticipantSettings[],
	right: readonly ChannelMixParticipantSettings[],
): boolean {
	const a = cleanSettings(left);
	const b = cleanSettings(right);
	if (a.length !== b.length) return false;
	return a.every(
		(item, index) =>
			item.user_id === b[index]?.user_id &&
			item.muted === b[index]?.muted &&
			Math.abs(item.gain_db - (b[index]?.gain_db ?? 0)) < 0.0001,
	);
}
