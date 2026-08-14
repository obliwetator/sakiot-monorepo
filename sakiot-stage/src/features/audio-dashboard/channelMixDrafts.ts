import type {
	ChannelMixParticipantSettings,
	ChannelMixTrack,
} from "../../app/apiSlice";
import { clampChannelMixGain } from "./channelMixState";

const STORAGE_KEY = "sakiot.channel-mix.drafts.v1";

type DraftStore = Record<string, ChannelMixParticipantSettings[]>;

function readStore(): DraftStore {
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return {};
		const parsed: unknown = JSON.parse(raw);
		if (!parsed || typeof parsed !== "object") return {};
		return parsed as DraftStore;
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
