import type {
	BaseQueryFn,
	FetchArgs,
	FetchBaseQueryError,
} from "@reduxjs/toolkit/query/react";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { components } from "../api/openapi";
import type { Channels, UserGuilds } from "../Constants";
import type { JamItRespStatus } from "../features/audio-dashboard/RangeSlider/JamIt";
import { BASE_API_URL, ensureRefreshed, getCsrfToken } from "./authedFetch";

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
	fetchFn: (input, init) => fetch(input, { ...init, credentials: "include" }),
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

export interface WaveformResponse {
	progress: number;
	data?: string;
	error?: string;
}

export type StampData = ApiSchema["StampInfo"];

export const apiSlice = createApi({
	reducerPath: "api",
	baseQuery: baseQueryWithReauth,
	tagTypes: ["Clips", "GuildCooldown", "UserOverrides"],
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
				url: `remove_silence/${guild_id}/${channel_id}/${year}/${month}/${file_name}`,
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
		getCurrentGuildDirs: builder.query<Channels[], string>({
			query: (guild_id) => `current/${guild_id}`,
		}),
		getLiveStems: builder.query<string[], string>({
			query: (guild_id) => `current/${guild_id}/live-stems`,
		}),
		getClips: builder.query<ClipData[], string>({
			query: (guild_id) => ({
				url: `audio/clips/${guild_id}`,
			}),
			providesTags: ["Clips"],
		}),
		getStamps: builder.query<StampData[], string>({
			query: (guild_id) => ({
				url: `stamps/${guild_id}`,
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
				url: `audio/clips/create/${guild_id}/${channel_id}/${year}/${month}/${file_name}`,
				method: "POST",
				headers: {
					Accept: "application/json",
					"Content-Type": "application/json",
				},
				body: { start, end, name },
			}),
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
	useGetClipsQuery,
	useDeleteClipMutation,
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
} = apiSlice;
