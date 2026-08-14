import type {
	BaseQueryFn,
	FetchArgs,
	FetchBaseQueryError,
} from "@reduxjs/toolkit/query/react";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { components } from "../api/openapi";
import type { Channels, UserGuilds } from "../Constants";
import type { JamItRespStatus } from "../features/audio-dashboard/RangeSlider/JamIt";
import {
	BASE_API_URL,
	captureCsrfToken,
	ensureRefreshed,
	getCsrfToken,
} from "./authedFetch";

export { BASE_API_URL };

type ApiSchema = components["schemas"];

export type User = ApiSchema["UserDataForFrontEnd"];

export type AuthDetails = {
	user: User | null;
	guilds: UserGuilds[] | null;
};

export type UserOverride = ApiSchema["UserOverride"];

export type RemoveSilenceResponse = ApiSchema["RemoveSilenceResponse"];

export type CreateClipResponse = ApiSchema["CreateClipResponse"];

const baseQuery = fetchBaseQuery({
	baseUrl: BASE_API_URL,
	fetchFn: async (input, init) => {
		const response = await fetch(input, { ...init, credentials: "include" });
		captureCsrfToken(response);
		return response;
	},
});

function withCsrfHeader(args: string | FetchArgs): string | FetchArgs {
	if (typeof args === "string") return args;

	const method = (args.method || "GET").toUpperCase();
	if (method === "GET" || method === "HEAD") return args;

	const csrf = getCsrfToken();
	if (!csrf) return args;

	const headers = new Headers(args.headers as HeadersInit | undefined);
	headers.set("X-CSRF-Token", csrf);
	return { ...args, headers };
}

const baseQueryWithReauth: BaseQueryFn<
	string | FetchArgs,
	unknown,
	FetchBaseQueryError
> = async (args, api, extraOptions) => {
	let result = await baseQuery(withCsrfHeader(args), api, extraOptions);

	if (result.error && result.error.status === 401) {
		const ok = await ensureRefreshed();
		if (ok) result = await baseQuery(withCsrfHeader(args), api, extraOptions);
	}
	return result;
};

export type ClipData = ApiSchema["ClipInfo"];

export type VoiceEvent = ApiSchema["VoiceEventDto"];
export type SessionManifest = ApiSchema["SessionManifestDto"];
export type SessionSegment = ApiSchema["SessionSegmentDto"];
export type SessionTimelineEvent = ApiSchema["SessionTimelineEventDto"];
export type GuildVoiceSettings = ApiSchema["GuildVoiceSettings"];

export type SessionWaveformResponse = ApiSchema["SessionWaveformResponse"];
export type ChannelMixResponse = ApiSchema["ChannelMixResponse"];
export type ChannelMixScope = ApiSchema["ChannelMixScope"];
export type ChannelMixTrack = ApiSchema["ChannelMixTrack"];
export type ChannelMixSourceSegment = ApiSchema["ChannelMixSourceSegment"];
export type ChannelMixParticipantSettings =
	ApiSchema["ChannelMixParticipantSettings"];
export type ChannelMixGenerationSettings =
	ApiSchema["ChannelMixGenerationSettings"];
export type GenerateChannelMixBody = ApiSchema["GenerateChannelMixBody"];

export interface WaveformResponse {
	progress: number;
	data?: string;
	error?: string;
}

export type StampData = ApiSchema["StampInfo"];

export type GuildRole = ApiSchema["GuildRole"];

export type RoleMember = ApiSchema["RoleMember"];

export type RoleView = ApiSchema["RoleView"];

export const apiSlice = createApi({
	reducerPath: "api",
	baseQuery: baseQueryWithReauth,
	tagTypes: ["Clips", "GuildCooldown", "UserOverrides", "GuildVoiceSettings"],
	endpoints: (builder) => ({
		jamIt: builder.mutation<
			{ code: JamItRespStatus },
			{ guild_id: string; clip_name: string }
		>({
			query: (body) => ({
				url: "jamit",
				method: "POST",
				headers: {
					Accept: "application/json",
					"Content-Type": "application/json",
				},
				body,
			}),
		}),
		removeSilence: builder.mutation<
			RemoveSilenceResponse,
			{
				guild_id: string;
				channel_id: string;
				year: string;
				month: number;
				file_name: string;
				idempotency_key: string;
			}
		>({
			query: ({
				guild_id,
				channel_id,
				year,
				month,
				file_name,
				idempotency_key,
			}) => ({
				url: `remove_silence/${guild_id}/${channel_id}/${year}/${month}/${encodeURIComponent(file_name)}`,
				method: "POST",
				headers: {
					Accept: "application/json",
					"Content-Type": "application/json",
					"Idempotency-Key": idempotency_key,
				},
			}),
		}),
		refresh: builder.mutation<void, void>({
			query: () => ({ url: "refresh", method: "POST" }),
		}),
		logout: builder.mutation<void, void>({
			query: () => ({ url: "logout", method: "POST" }),
		}),
		getCurrentGuildDirs: builder.query<
			Channels[],
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) =>
				`current/${guild_id}${as_role ? `?as_role=${as_role}` : ""}`,
		}),
		getLiveStems: builder.query<
			string[],
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) =>
				`current/${guild_id}/live-stems${as_role ? `?as_role=${as_role}` : ""}`,
		}),
		getSessionManifest: builder.query<SessionManifest, string>({
			query: (recording_session_id) =>
				`audio/sessions/${recording_session_id}/manifest`,
		}),
		getSessionChannelMix: builder.query<
			ChannelMixResponse,
			{ recording_session_id: string; scope?: ChannelMixScope }
		>({
			query: ({ recording_session_id, scope = "all_recordings" }) =>
				`audio/sessions/${recording_session_id}/channel-mix?scope=${scope}`,
		}),
		generateSessionChannelMix: builder.mutation<
			ChannelMixResponse,
			{
				recording_session_id: string;
				scope?: ChannelMixScope;
				body?: GenerateChannelMixBody;
			}
		>({
			query: ({ recording_session_id, scope = "all_recordings", body }) => ({
				url: `audio/sessions/${recording_session_id}/channel-mix?scope=${scope}`,
				method: "POST",
				...(body ? { body } : {}),
			}),
		}),
		getSessionWaveform: builder.query<SessionWaveformResponse, string>({
			query: (recording_session_id) =>
				`audio/sessions/${recording_session_id}/waveform`,
		}),
		getSilenceFreeSessionWaveform: builder.query<
			SessionWaveformResponse,
			string
		>({
			query: (recording_session_id) =>
				`audio/sessions/${recording_session_id}/silence-free/waveform`,
		}),
		rebuildSessionWaveform: builder.mutation<SessionWaveformResponse, string>({
			query: (recording_session_id) => ({
				url: `audio/sessions/${recording_session_id}/waveform/rebuild`,
				method: "POST",
			}),
		}),
		rebuildSilenceFreeSessionWaveform: builder.mutation<
			SessionWaveformResponse,
			string
		>({
			query: (recording_session_id) => ({
				url: `audio/sessions/${recording_session_id}/silence-free/waveform/rebuild`,
				method: "POST",
			}),
		}),
		createSessionClip: builder.mutation<
			CreateClipResponse,
			{
				recording_session_id: string;
				start: number;
				end: number;
				name?: string;
				silence_free?: boolean;
			}
		>({
			query: ({ recording_session_id, start, end, name, silence_free }) => ({
				url: `audio/sessions/${recording_session_id}/clips`,
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: { start, end, name, silence_free },
			}),
			invalidatesTags: ["Clips"],
		}),
		getClips: builder.query<ClipData[], { guild_id: string; as_role?: string }>(
			{
				query: ({ guild_id, as_role }) => ({
					url: `audio/clips/${guild_id}${as_role ? `?as_role=${as_role}` : ""}`,
				}),
				providesTags: ["Clips"],
			},
		),
		composeClip: builder.mutation<
			ApiSchema["ComposeClipAccepted"],
			{ guild_id: string; body: ApiSchema["ComposeClipBody"] }
		>({
			query: ({ guild_id, body }) => ({
				url: `audio/clips/${guild_id}/compose`,
				method: "POST",
				headers: {
					Accept: "application/json",
					"Content-Type": "application/json",
				},
				body,
			}),
		}),
		getComposeClipStatus: builder.query<
			ApiSchema["ComposeClipStatus"],
			{ guild_id: string; clip_id: string }
		>({
			query: ({ guild_id, clip_id }) => ({
				url: `audio/clips/${guild_id}/compose/${encodeURIComponent(clip_id)}`,
			}),
		}),
		getStamps: builder.query<
			StampData[],
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) => ({
				url: `stamps/${guild_id}${as_role ? `?as_role=${as_role}` : ""}`,
			}),
		}),
		deleteClip: builder.mutation<void, { guild_id: string; file_name: string }>(
			{
				query: ({ guild_id, file_name }) => ({
					url: `audio/clips/${guild_id}/${encodeURIComponent(file_name)}`,
					method: "DELETE",
				}),
				invalidatesTags: ["Clips"],
			},
		),
		renameClip: builder.mutation<
			void,
			{ guild_id: string; clip_id: string; name: string }
		>({
			query: ({ guild_id, clip_id, name }) => ({
				url: `audio/clips/${guild_id}/${encodeURIComponent(clip_id)}`,
				method: "PUT",
				headers: { "Content-Type": "application/json" },
				body: { name },
			}),
			invalidatesTags: ["Clips"],
		}),
		createClip: builder.mutation<
			CreateClipResponse,
			{
				guild_id: string;
				channel_id: string;
				year: string;
				month: number;
				file_name: string;
				start: number;
				end: number;
				name?: string;
			}
		>({
			query: ({
				guild_id,
				channel_id,
				year,
				month,
				file_name,
				start,
				end,
				name,
			}) => ({
				url: `audio/clips/create/${guild_id}/${channel_id}/${year}/${month}/${encodeURIComponent(file_name)}`,
				method: "POST",
				headers: {
					Accept: "application/json",
					"Content-Type": "application/json",
				},
				body: { start, end, name },
			}),
			invalidatesTags: ["Clips"],
		}),
		checkSilenceFile: builder.query<
			void,
			{
				guild_id: string;
				channel_id: string;
				year: string;
				month: number;
				file_name: string;
			}
		>({
			query: ({ guild_id, channel_id, year, month, file_name }) => ({
				url: `audio/${guild_id}/${channel_id}/${year}/${month}/${encodeURIComponent(file_name)}.ogg?silence=true`,
				method: "HEAD",
			}),
		}),
		getRecordingEvents: builder.query<
			VoiceEvent[],
			{
				guild_id: string;
				channel_id: string;
				year: string;
				month: number;
				file_name: string;
				user_id?: string;
			}
		>({
			query: ({ guild_id, channel_id, year, month, file_name, user_id }) => ({
				url: `audio/events/${guild_id}/${channel_id}/${year}/${month}/${encodeURIComponent(file_name)}${user_id ? `?user_id=${user_id}` : ""}`,
			}),
		}),
		getLiveState: builder.query<
			{ live: boolean; started_at: number | null; ended_at: number | null },
			{
				guild_id: string;
				channel_id: string;
				year: string;
				month: number;
				file_name: string;
			}
		>({
			query: ({ guild_id, channel_id, year, month, file_name }) => ({
				url: `audio/live/${guild_id}/${channel_id}/${year}/${month}/${encodeURIComponent(file_name)}/state`,
			}),
		}),
		getWaveform: builder.query<
			WaveformResponse,
			{
				guild_id: string;
				channel_id: string;
				year: string;
				month: number;
				file_name: string;
				timestamp?: number;
				silence?: boolean;
			}
		>({
			query: ({
				guild_id,
				channel_id,
				year,
				month,
				file_name,
				timestamp,
				silence,
			}) => {
				const qs = new URLSearchParams();
				if (timestamp) qs.set("t", String(timestamp));
				if (silence) qs.set("silence", "true");
				const suffix = qs.toString() ? `?${qs}` : "";
				return {
					url: `audio/waveform/${guild_id}/${channel_id}/${year}/${month}/${encodeURIComponent(file_name)}${suffix}`,
				};
			},
		}),
		// Clips are their own trimmed file, keyed by clip_id — separate endpoint
		// from the recording waveform (which needs channel_id/year/month).
		getClipWaveform: builder.query<
			WaveformResponse,
			{ guild_id: string; clip_id: string; timestamp?: number }
		>({
			query: ({ guild_id, clip_id, timestamp }) => {
				const qs = new URLSearchParams();
				if (timestamp) qs.set("t", String(timestamp));
				const suffix = qs.toString() ? `?${qs}` : "";
				return {
					url: `audio/clips/waveform/${guild_id}/${encodeURIComponent(clip_id)}${suffix}`,
				};
			},
		}),
		getGuildCooldown: builder.query<ApiSchema["GuildCooldown"], string>({
			query: (guild_id) => `admin/guilds/${guild_id}/cooldown`,
			providesTags: (_r, _e, id) => [{ type: "GuildCooldown", id }],
		}),
		setGuildCooldown: builder.mutation<
			void,
			{ guild_id: string; cooldown_seconds: number }
		>({
			query: ({ guild_id, cooldown_seconds }) => ({
				url: `admin/guilds/${guild_id}/cooldown`,
				method: "PUT",
				headers: { "Content-Type": "application/json" },
				body: { cooldown_seconds },
			}),
			invalidatesTags: (_r, _e, { guild_id }) => [
				{ type: "GuildCooldown", id: guild_id },
			],
		}),
		listUserOverrides: builder.query<UserOverride[], string>({
			query: (guild_id) => `admin/guilds/${guild_id}/cooldown/overrides`,
			providesTags: (_r, _e, id) => [{ type: "UserOverrides", id }],
		}),
		setUserOverride: builder.mutation<
			void,
			{ guild_id: string; user_id: string; cooldown_seconds: number }
		>({
			query: ({ guild_id, user_id, cooldown_seconds }) => ({
				url: `admin/guilds/${guild_id}/cooldown/overrides/${user_id}`,
				method: "PUT",
				headers: { "Content-Type": "application/json" },
				body: { cooldown_seconds },
			}),
			invalidatesTags: (_r, _e, { guild_id }) => [
				{ type: "UserOverrides", id: guild_id },
			],
		}),
		deleteUserOverride: builder.mutation<
			void,
			{ guild_id: string; user_id: string }
		>({
			query: ({ guild_id, user_id }) => ({
				url: `admin/guilds/${guild_id}/cooldown/overrides/${user_id}`,
				method: "DELETE",
			}),
			invalidatesTags: (_r, _e, { guild_id }) => [
				{ type: "UserOverrides", id: guild_id },
			],
		}),
		getGuildVoiceSettings: builder.query<GuildVoiceSettings, string>({
			query: (guild_id) => `admin/guilds/${guild_id}/voice-settings`,
			providesTags: (_result, _error, guild_id) => [
				{ type: "GuildVoiceSettings", id: guild_id },
			],
		}),
		setGuildVoiceSettings: builder.mutation<
			GuildVoiceSettings,
			{ guild_id: string; pending_cap_seconds: number }
		>({
			query: ({ guild_id, pending_cap_seconds }) => ({
				url: `admin/guilds/${guild_id}/voice-settings`,
				method: "PUT",
				headers: { "Content-Type": "application/json" },
				body: { pending_cap_seconds },
			}),
			invalidatesTags: (_result, _error, { guild_id }) => [
				{ type: "GuildVoiceSettings", id: guild_id },
			],
		}),
		deleteGuildVoiceSettings: builder.mutation<GuildVoiceSettings, string>({
			query: (guild_id) => ({
				url: `admin/guilds/${guild_id}/voice-settings`,
				method: "DELETE",
			}),
			invalidatesTags: (_result, _error, guild_id) => [
				{ type: "GuildVoiceSettings", id: guild_id },
			],
		}),
		getGuildRoles: builder.query<GuildRole[], string>({
			query: (guild_id) => `admin/guilds/${guild_id}/roles`,
		}),
		getRoleMembers: builder.query<
			RoleMember[],
			{ guild_id: string; role_id: string }
		>({
			query: ({ guild_id, role_id }) =>
				`admin/guilds/${guild_id}/roles/${role_id}/members`,
		}),
		getRoleView: builder.query<RoleView, { guild_id: string; role_id: string }>(
			{
				query: ({ guild_id, role_id }) =>
					`admin/guilds/${guild_id}/roles/${role_id}/channels`,
			},
		),
		// Combine all 3 requests into a single query to emulate the existing Promise.all behavior
		getAuthDetails: builder.query<AuthDetails, void>({
			async queryFn(_arg, _queryApi, _extraOptions, fetchWithBQ) {
				const [userResult, guildsResult] = await Promise.all([
					fetchWithBQ("users/current"),
					fetchWithBQ("users/current/guilds"),
				]);

				if (userResult.error || guildsResult.error) {
					return {
						error: (userResult.error ||
							guildsResult.error) as FetchBaseQueryError,
					};
				}

				return {
					data: {
						user: userResult.data as User,
						guilds: guildsResult.data as UserGuilds[],
					},
				};
			},
		}),
	}),
});

export const {
	useGetAuthDetailsQuery,
	useGetCurrentGuildDirsQuery,
	useGetLiveStemsQuery,
	useGetSessionManifestQuery,
	useGetSessionChannelMixQuery,
	useGenerateSessionChannelMixMutation,
	useGetSessionWaveformQuery,
	useGetSilenceFreeSessionWaveformQuery,
	useRebuildSessionWaveformMutation,
	useRebuildSilenceFreeSessionWaveformMutation,
	useCreateSessionClipMutation,
	useGetClipsQuery,
	useComposeClipMutation,
	useGetComposeClipStatusQuery,
	useDeleteClipMutation,
	useRenameClipMutation,
	useJamItMutation,
	useRemoveSilenceMutation,
	useCheckSilenceFileQuery,
	useRefreshMutation,
	useLogoutMutation,
	useCreateClipMutation,
	useGetLiveStateQuery,
	useGetRecordingEventsQuery,
	useGetWaveformQuery,
	useLazyGetWaveformQuery,
	useGetClipWaveformQuery,
	useGetStampsQuery,
	useGetGuildCooldownQuery,
	useSetGuildCooldownMutation,
	useListUserOverridesQuery,
	useSetUserOverrideMutation,
	useDeleteUserOverrideMutation,
	useGetGuildVoiceSettingsQuery,
	useSetGuildVoiceSettingsMutation,
	useDeleteGuildVoiceSettingsMutation,
	useGetGuildRolesQuery,
	useGetRoleMembersQuery,
	useGetRoleViewQuery,
} = apiSlice;
