import { BASE_API_URL } from "../app/authedFetch";
import type { paths } from "./openapi";

/**
 * Every path this client may call, written exactly as the backend documents it.
 *
 * The `satisfies` clause is the point: a route renamed in `web-server` changes
 * the generated `paths` type, so this file stops compiling until the client is
 * updated. Hand-written URL strings could not catch that.
 */
export const API_ROUTES = {
	jamIt: "/api/jamit",
	oauthStart: "/api/oauth/start",
	removeSilence:
		"/api/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
	refresh: "/api/refresh",
	logout: "/api/logout",
	currentGuildDirs: "/api/current/{guild_id}",
	liveStems: "/api/current/{guild_id}/live-stems",
	sessionManifest: "/api/audio/sessions/{recording_session_id}/manifest",
	sessionChannelMix: "/api/audio/sessions/{recording_session_id}/channel-mix",
	sessionWaveform: "/api/audio/sessions/{recording_session_id}/waveform",
	sessionSilenceFreeWaveform:
		"/api/audio/sessions/{recording_session_id}/silence-free/waveform",
	sessionSilenceFree: "/api/audio/sessions/{recording_session_id}/silence-free",
	sessionRemoveSilence:
		"/api/audio/sessions/{recording_session_id}/remove-silence",
	sessionDownload: "/api/audio/sessions/{recording_session_id}/download",
	sessionChannelMixMedia:
		"/api/audio/sessions/{recording_session_id}/channel-mix/media",
	sessionWaveformRebuild:
		"/api/audio/sessions/{recording_session_id}/waveform/rebuild",
	sessionSilenceFreeWaveformRebuild:
		"/api/audio/sessions/{recording_session_id}/silence-free/waveform/rebuild",
	sessionClips: "/api/audio/sessions/{recording_session_id}/clips",
	clips: "/api/audio/clips/{guild_id}",
	clip: "/api/audio/clips/{guild_id}/{clip_id}",
	clipCompose: "/api/audio/clips/{guild_id}/compose",
	clipComposeStatus: "/api/audio/clips/{guild_id}/compose/{clip_id}",
	clipCreate:
		"/api/audio/clips/create/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
	clipWaveform: "/api/audio/clips/waveform/{guild_id}/{clip_id}",
	stamps: "/api/stamps/{guild_id}",
	audio: "/api/audio/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
	recordingEvents:
		"/api/audio/events/{guild_id}/{channel_id}/{year}/{month}/{stem}",
	recordingWaveform:
		"/api/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}",
	liveState:
		"/api/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/state",
	livePlaylist:
		"/api/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/playlist.m3u8",
	guildCooldown: "/api/admin/guilds/{guild_id}/cooldown",
	userOverrides: "/api/admin/guilds/{guild_id}/cooldown/overrides",
	userOverride: "/api/admin/guilds/{guild_id}/cooldown/overrides/{user_id}",
	guildVoiceSettings: "/api/admin/guilds/{guild_id}/voice-settings",
	guildRoles: "/api/admin/guilds/{guild_id}/roles",
	roleMembers: "/api/admin/guilds/{guild_id}/roles/{role_id}/members",
	roleChannels: "/api/admin/guilds/{guild_id}/roles/{role_id}/channels",
	currentUser: "/api/users/current",
	currentUserGuilds: "/api/users/current/guilds",
} satisfies Record<string, keyof paths & string>;

export type ApiRoute = (typeof API_ROUTES)[keyof typeof API_ROUTES];

const API_SUFFIX = "/api/";

/**
 * Builds an RTK Query `url` for a documented route.
 *
 * `fetchBaseQuery` resolves `url` against `VITE_API_URL`, which points at the
 * API root (…/api/), so the `/api/` prefix is stripped when the base already
 * carries it. Every `{placeholder}` must be supplied: an unsubstituted one is a
 * caller bug, not a valid request.
 */
export function apiUrl(
	route: ApiRoute,
	params?: Record<string, string | number>,
): string {
	let path: string = route;

	for (const [name, value] of Object.entries(params ?? {})) {
		path = path.replace(`{${name}}`, encodeURIComponent(String(value)));
	}

	const unresolved = path.match(/\{[^}]+\}/);
	if (unresolved) {
		throw new Error(`apiUrl: missing value for ${unresolved[0]} in ${route}`);
	}

	return BASE_API_URL.endsWith(API_SUFFIX)
		? path.slice(API_SUFFIX.length)
		: path;
}

/**
 * Absolute URL for a documented route. Media elements (`<audio src>`, HLS
 * playlists) need a full origin, unlike `apiUrl`'s relative fetch path.
 */
export function apiAbsoluteUrl(
	route: ApiRoute,
	params?: Record<string, string | number>,
): string {
	const base = BASE_API_URL.endsWith("/") ? BASE_API_URL : `${BASE_API_URL}/`;
	return `${base}${apiUrl(route, params)}`;
}
