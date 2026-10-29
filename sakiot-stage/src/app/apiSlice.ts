import type {
	BaseQueryFn,
	FetchArgs,
	FetchBaseQueryError,
} from "@reduxjs/toolkit/query/react";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { components } from "../api/openapi";
import { API_ROUTES, apiUrl } from "../api/routes";
import type { Channels, UserGuilds } from "../Constants";
import {
	BASE_API_URL,
	currentAccountEpoch,
	getCsrfToken,
	healCsrfToken,
	refreshSession,
} from "./authedFetch";
import type { MediaJobStatus } from "./mediaJobs";
import { retryWhileMembershipLoads } from "./membershipRetry";

export { BASE_API_URL };

type ApiSchema = components["schemas"];

export type User = ApiSchema["UserDataForFrontEnd"];

export type AuthDetails = {
	user: User | null;
	guilds: UserGuilds[] | null;
};

export type UserOverride = ApiSchema["UserOverride"];

export type RemoveSilenceResponse =
	| ApiSchema["RemoveSilenceResponse"]
	| MediaJobStatus;

export type CreateClipResponse = ApiSchema["CreateClipResponse"];

export type PlayClipResponse = ApiSchema["PlayClipResponse"];

export type SessionInfo = ApiSchema["SessionInfo"];

const baseQuery = fetchBaseQuery({
	baseUrl: BASE_API_URL,
	fetchFn: async (input, init) => {
		const epoch = currentAccountEpoch();
		const response = await fetch(input, { ...init, credentials: "include" });
		healCsrfToken(response, epoch);
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

/**
 * Set once per expiry episode so the invalidated auth probe, which itself 401s
 * and fails its own refresh, cannot trigger another invalidation. A successful
 * refresh (a new session) clears it.
 */
let sessionExpiryHandled = false;

/** Reads may be repeated; writes never are. */
function isRead(args: string | FetchArgs): boolean {
	if (typeof args === "string") return true;
	const method = (args.method || "GET").toUpperCase();
	return method === "GET" || method === "HEAD";
}

/** The auth probe: a success means the document now has a live session. */
function isAuthProbe(args: string | FetchArgs): boolean {
	const url = typeof args === "string" ? args : args.url;
	return url.endsWith("users/current");
}

const baseQueryWithReauth: BaseQueryFn<
	string | FetchArgs,
	unknown,
	FetchBaseQueryError
> = async (args, api, extraOptions) => {
	const request = () => baseQuery(withCsrfHeader(args), api, extraOptions);
	let result = isRead(args)
		? await retryWhileMembershipLoads(request, api.signal)
		: await request();

	// A successful auth probe means a session was re-established inside this
	// document (OAuth popup or dev login without a reload), so the next expiry
	// must be able to flip the shell again.
	if (!result.error && sessionExpiryHandled && isAuthProbe(args)) {
		sessionExpiryHandled = false;
	}

	if (result.error && result.error.status === 401) {
		const outcome = await refreshSession();
		if (outcome === "refreshed") {
			sessionExpiryHandled = false;
			result = await baseQuery(withCsrfHeader(args), api, extraOptions);
		} else if (outcome === "session-ended" && !sessionExpiryHandled) {
			// A network error or 5xx from /refresh is temporary: the request
			// fails, but the session (and any drafts) stay.
			// The refresh token is gone too: the session is over. Drop the
			// cached auth details so the shell renders the logged-out state
			// instead of a logged-in UI whose every action 401s.
			sessionExpiryHandled = true;
			api.dispatch(apiSlice.util.invalidateTags(["Auth"]));
		}
	}
	return result;
};

export type ClipData = ApiSchema["ClipInfo"];

export type VoiceEvent = ApiSchema["VoiceEventDto"];
export type SessionManifest = ApiSchema["SessionManifestDto"];
export type SessionSegment = ApiSchema["SessionSegmentDto"];
export type GuildVoiceSettings = ApiSchema["GuildVoiceSettings"];
export type GuildRecordingPolicy = ApiSchema["GuildRecordingPolicy"];
export type RecordingOptOut = ApiSchema["RecordingOptOut"];
export type RecordingPrivacy = ApiSchema["RecordingPrivacy"];
export type RecordingDeletionStatus = ApiSchema["RecordingDeletionStatus"];
export type ErasureStatus = ApiSchema["ErasureStatus"];
/** Why one of the viewer's own sessions is hidden from everyone else. */
export type Removal = ApiSchema["Removal"];

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
	/** The media job building it, until the data is ready (a 202 job status). */
	id?: string;
	/**
	 * False while a live recording has no waveform. The recorder writes one as
	 * it records, so this means it failed or predates that; the waveform is
	 * built from the audio once the recording ends.
	 */
	built?: false;
}

export type StampData = ApiSchema["StampInfo"];

export type GuildRole = ApiSchema["GuildRole"];

export type RoleMember = ApiSchema["RoleMember"];
export type MemberPage = ApiSchema["MemberPage"];
export type GuildMember = ApiSchema["GuildMember"];
export type VoicePresence = ApiSchema["VoicePresence"];

export type RoleView = ApiSchema["RoleView"];

export type PermissionRules = ApiSchema["PermissionRules"];
export type PermissionRule = ApiSchema["PermissionRule"];
export type RuleSubject = ApiSchema["RuleSubject"];
export type RuleChange = ApiSchema["RuleChange"];
export type RuleRole = ApiSchema["RuleRole"];
export type AccessExplanation = ApiSchema["AccessExplanation"];
export type CapabilityDecision = ApiSchema["CapabilityDecision"];

export type Keeping = ApiSchema["Keeping"];
export type KeptRecording = ApiSchema["KeptRecording"];
export type RemovedRecordings = ApiSchema["RemovedRecordings"];
export type RemovedRecording = ApiSchema["RemovedRecording"];
export type KeepAllowances = ApiSchema["KeepAllowances"];
export type KeepAllowance = ApiSchema["KeepAllowance"];
export type KeepAllowanceChange = ApiSchema["KeepAllowanceChange"];
export type OperatorGuild = ApiSchema["OperatorGuild"];

/** Two constants, not one: unifying them would change what requests send. */
const JSON_BODY_HEADERS = { "Content-Type": "application/json" } as const;
const JSON_HEADERS = {
	Accept: "application/json",
	...JSON_BODY_HEADERS,
} as const;

const TAG_TYPES = [
	"Auth",
	"Sessions",
	"Clips",
	"GuildCooldown",
	"UserOverrides",
	"GuildVoiceSettings",
	"GuildRecordingPolicy",
	"RecordingOptOut",
	/** Who may hear the viewer's own recordings in the guild. */
	"RecordingPrivacy",
	"Recordings",
	"LiveStems",
	"LiveState",
	"Stamps",
	"GuildRoles",
	/** Member search over the guild roster. */
	"GuildMembers",
	/** Who is in the guild's voice and stage channels. */
	"VoicePresence",
	/** Sakiot's own permission rules, as the rule editor shows them. */
	"PermissionRules",
	/**
	 * What a member or a role holds, and what decided it: the rule
	 * explainer and the Members page's role preview.
	 */
	"PermissionExplanation",
	/** The guild's cap, the viewer's allowance and the kept recordings. */
	"Keeping",
	/** Removed recordings the viewer may keep or restore. */
	"RemovedRecordings",
	/** How many recordings each member may keep, as managers set it. */
	"KeepAllowances",
	/** Every guild's limits, for the bot's operators. */
	"OperatorGuilds",
	/** One recording session's manifest and waveform, by session id. */
	"Session",
	/**
	 * A background job a page is waiting on, by job id: realtime `jobs`
	 * events refresh exactly the queries still waiting on that job.
	 */
	"Job",
] as const;
export type ApiTag = (typeof TAG_TYPES)[number];

/** The `Job` tag of a response that is still waiting on a job, if any. */
const jobTags = (jobId: string | null | undefined) =>
	jobId ? [{ type: "Job" as const, id: jobId }] : [];

/** Tags for anything scoped to one guild, given the id bare or in an object. */
const guildScoped =
	(...types: ApiTag[]) =>
	(_result: unknown, _error: unknown, arg: string | { guild_id: string }) => {
		const id = typeof arg === "string" ? arg : arg.guild_id;
		return types.map((type) => ({ type, id }));
	};

/** Append the admin "view as role" query parameter when one is set. */
const withAsRole = (url: string, as_role?: string): string =>
	`${url}${as_role ? `?as_role=${as_role}` : ""}`;

const sameSubject = (a: RuleSubject, b: RuleSubject) =>
	a.kind === b.kind && (a.id ?? null) === (b.id ?? null);

/**
 * Replaces the entry `matches` finds with what `write` makes of it (adding
 * one when none matches), or removes it when `write` gives null.
 */
function writeEntry<T>(
	list: T[],
	matches: (entry: T) => boolean,
	write: (entry: T | undefined) => T | null,
) {
	const at = list.findIndex(matches);
	const next = write(at === -1 ? undefined : list[at]);
	if (at === -1) {
		if (next) list.push(next);
	} else if (next) list[at] = next;
	else list.splice(at, 1);
}

export const apiSlice = createApi({
	reducerPath: "api",
	baseQuery: baseQueryWithReauth,
	tagTypes: TAG_TYPES,
	endpoints: (builder) => ({
		playClip: builder.mutation<
			PlayClipResponse,
			{ guild_id: string; clip_id: string }
		>({
			query: ({ guild_id, clip_id }) => ({
				url: apiUrl(API_ROUTES.clipPlay, { guild_id, clip_id }),
				method: "POST",
				headers: { Accept: "application/json" },
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
				url: apiUrl(API_ROUTES.removeSilence, {
					guild_id,
					channel_id,
					year,
					month,
					file_name,
				}),
				method: "POST",
				headers: {
					Accept: "application/json",
					"Content-Type": "application/json",
					"Idempotency-Key": idempotency_key,
				},
			}),
		}),
		refresh: builder.mutation<void, void>({
			query: () => ({ url: apiUrl(API_ROUTES.refresh), method: "POST" }),
		}),
		logout: builder.mutation<void, void>({
			query: () => ({ url: apiUrl(API_ROUTES.logout), method: "POST" }),
		}),
		getSessions: builder.query<SessionInfo[], void>({
			query: () => apiUrl(API_ROUTES.currentUserSessions),
			providesTags: ["Sessions"],
		}),
		/** Ends one session; that browser stops at its next refresh. */
		endSession: builder.mutation<void, string>({
			query: (session_id) => ({
				url: apiUrl(API_ROUTES.currentUserSession, { session_id }),
				method: "DELETE",
			}),
			invalidatesTags: ["Sessions"],
		}),
		/** Ends every session of the account; other browsers stop at their next refresh. */
		logoutEverywhere: builder.mutation<void, void>({
			query: () => ({
				url: apiUrl(API_ROUTES.currentUserSessions),
				method: "DELETE",
			}),
		}),
		getCurrentGuildDirs: builder.query<
			Channels[],
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) =>
				withAsRole(apiUrl(API_ROUTES.currentGuildDirs, { guild_id }), as_role),
			providesTags: guildScoped("Recordings"),
		}),
		getLiveStems: builder.query<
			string[],
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) =>
				withAsRole(apiUrl(API_ROUTES.liveStems, { guild_id }), as_role),
			providesTags: guildScoped("LiveStems"),
		}),
		/**
		 * One session's recording-tree entry (null when missing or not visible),
		 * for patching cached trees after a realtime event.
		 */
		getSessionListingEntry: builder.query<
			Channels | null,
			{ guild_id: string; recording_session_id: string; as_role?: string }
		>({
			query: ({ guild_id, recording_session_id, as_role }) =>
				withAsRole(
					apiUrl(API_ROUTES.sessionListingEntry, {
						guild_id,
						recording_session_id,
					}),
					as_role,
				),
			keepUnusedDataFor: 0,
		}),
		getSessionManifest: builder.query<SessionManifest, string>({
			query: (recording_session_id) =>
				apiUrl(API_ROUTES.sessionManifest, { recording_session_id }),
			providesTags: (_result, _error, id) => [{ type: "Session", id }],
		}),
		getSessionChannelMix: builder.query<
			ChannelMixResponse,
			{ recording_session_id: string; scope?: ChannelMixScope }
		>({
			query: ({ recording_session_id, scope = "all_recordings" }) =>
				`${apiUrl(API_ROUTES.sessionChannelMix, { recording_session_id })}?scope=${scope}`,
			providesTags: (result) => jobTags(result?.job_id),
		}),
		generateSessionChannelMix: builder.mutation<
			ChannelMixResponse | MediaJobStatus,
			{
				recording_session_id: string;
				scope?: ChannelMixScope;
				body?: GenerateChannelMixBody;
			}
		>({
			query: ({ recording_session_id, scope = "all_recordings", body }) => ({
				url: `${apiUrl(API_ROUTES.sessionChannelMix, { recording_session_id })}?scope=${scope}`,
				method: "POST",
				...(body ? { body } : {}),
			}),
		}),
		getSessionWaveform: builder.query<SessionWaveformResponse, string>({
			query: (recording_session_id) =>
				apiUrl(API_ROUTES.sessionWaveform, { recording_session_id }),
			providesTags: (result, _error, id) => [
				{ type: "Session", id },
				...jobTags(result?.job_id),
			],
		}),
		getSilenceFreeSessionWaveform: builder.query<
			SessionWaveformResponse,
			string
		>({
			query: (recording_session_id) =>
				apiUrl(API_ROUTES.sessionSilenceFreeWaveform, {
					recording_session_id,
				}),
			providesTags: (result) => jobTags(result?.job_id),
		}),
		rebuildSessionWaveform: builder.mutation<MediaJobStatus, string>({
			query: (recording_session_id) => ({
				url: apiUrl(API_ROUTES.sessionWaveformRebuild, {
					recording_session_id,
				}),
				method: "POST",
			}),
		}),
		rebuildSilenceFreeSessionWaveform: builder.mutation<MediaJobStatus, string>(
			{
				query: (recording_session_id) => ({
					url: apiUrl(API_ROUTES.sessionSilenceFreeWaveformRebuild, {
						recording_session_id,
					}),
					method: "POST",
				}),
			},
		),
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
				url: apiUrl(API_ROUTES.sessionClips, { recording_session_id }),
				method: "POST",
				headers: JSON_BODY_HEADERS,
				body: { start, end, name, silence_free },
			}),
			invalidatesTags: ["Clips"],
		}),
		getClips: builder.query<ClipData[], { guild_id: string; as_role?: string }>(
			{
				query: ({ guild_id, as_role }) => ({
					url: withAsRole(apiUrl(API_ROUTES.clips, { guild_id }), as_role),
				}),
				providesTags: guildScoped("Clips"),
			},
		),
		composeClip: builder.mutation<
			ApiSchema["ComposeClipAccepted"],
			{
				guild_id: string;
				body: ApiSchema["ComposeClipBody"];
				idempotency_key: string;
			}
		>({
			query: ({ guild_id, body, idempotency_key }) => ({
				url: apiUrl(API_ROUTES.clipCompose, { guild_id }),
				method: "POST",
				headers: {
					"Idempotency-Key": idempotency_key,
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
				url: apiUrl(API_ROUTES.clipComposeStatus, { guild_id, clip_id }),
			}),
			// `clip_id` is the export job's id.
			providesTags: (_result, _error, { clip_id }) => jobTags(clip_id),
		}),
		getStamps: builder.query<
			StampData[],
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) => ({
				url: withAsRole(apiUrl(API_ROUTES.stamps, { guild_id }), as_role),
			}),
			providesTags: guildScoped("Stamps"),
		}),
		deleteClip: builder.mutation<void, { guild_id: string; file_name: string }>(
			{
				query: ({ guild_id, file_name }) => ({
					url: apiUrl(API_ROUTES.clip, { guild_id, clip_id: file_name }),
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
				url: apiUrl(API_ROUTES.clip, { guild_id, clip_id }),
				method: "PUT",
				headers: JSON_BODY_HEADERS,
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
				url: apiUrl(API_ROUTES.clipCreate, {
					guild_id,
					channel_id,
					year,
					month,
					file_name,
				}),
				method: "POST",
				headers: JSON_HEADERS,
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
				url: `${apiUrl(API_ROUTES.audio, {
					guild_id,
					channel_id,
					year,
					month,
					file_name: `${file_name}.ogg`,
				})}?silence=true`,
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
				url: `${apiUrl(API_ROUTES.recordingEvents, {
					guild_id,
					channel_id,
					year,
					month,
					stem: file_name,
				})}${user_id ? `?user_id=${user_id}` : ""}`,
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
				url: apiUrl(API_ROUTES.liveState, {
					guild_id,
					channel_id,
					year,
					month,
					stem: file_name,
				}),
			}),
			providesTags: guildScoped("LiveState"),
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
					url: `${apiUrl(API_ROUTES.recordingWaveform, {
						guild_id,
						channel_id,
						year,
						month,
						file: file_name,
					})}${suffix}`,
				};
			},
			providesTags: (result) => jobTags(result?.id),
		}),
		// Channel-mix source metadata already contains the canonical root-relative
		// waveform URL. Keep those requests in RTK Query's cache too so the same
		// physical source can be rendered in both the track and source rows without
		// starting another fetch (or another server-side build).
		getWaveformByUrl: builder.query<WaveformResponse, string>({
			query: (url) => url.replace(/^\/api\//, ""),
			providesTags: (result) => jobTags(result?.id),
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
					url: `${apiUrl(API_ROUTES.clipWaveform, { guild_id, clip_id })}${suffix}`,
				};
			},
			providesTags: (result) => jobTags(result?.id),
		}),
		getGuildCooldown: builder.query<ApiSchema["GuildCooldown"], string>({
			query: (guild_id) => apiUrl(API_ROUTES.guildCooldown, { guild_id }),
			providesTags: guildScoped("GuildCooldown"),
		}),
		setGuildCooldown: builder.mutation<
			void,
			{ guild_id: string; cooldown_seconds: number }
		>({
			query: ({ guild_id, cooldown_seconds }) => ({
				url: apiUrl(API_ROUTES.guildCooldown, { guild_id }),
				method: "PUT",
				headers: JSON_BODY_HEADERS,
				body: { cooldown_seconds },
			}),
			invalidatesTags: guildScoped("GuildCooldown"),
		}),
		listUserOverrides: builder.query<UserOverride[], string>({
			query: (guild_id) => apiUrl(API_ROUTES.userOverrides, { guild_id }),
			providesTags: guildScoped("UserOverrides"),
		}),
		setUserOverride: builder.mutation<
			void,
			{ guild_id: string; user_id: string; cooldown_seconds: number }
		>({
			query: ({ guild_id, user_id, cooldown_seconds }) => ({
				url: apiUrl(API_ROUTES.userOverride, { guild_id, user_id }),
				method: "PUT",
				headers: JSON_BODY_HEADERS,
				body: { cooldown_seconds },
			}),
			invalidatesTags: guildScoped("UserOverrides"),
		}),
		deleteUserOverride: builder.mutation<
			void,
			{ guild_id: string; user_id: string }
		>({
			query: ({ guild_id, user_id }) => ({
				url: apiUrl(API_ROUTES.userOverride, { guild_id, user_id }),
				method: "DELETE",
			}),
			invalidatesTags: guildScoped("UserOverrides"),
		}),
		getGuildVoiceSettings: builder.query<GuildVoiceSettings, string>({
			query: (guild_id) => apiUrl(API_ROUTES.guildVoiceSettings, { guild_id }),
			providesTags: guildScoped("GuildVoiceSettings"),
		}),
		setGuildVoiceSettings: builder.mutation<
			GuildVoiceSettings,
			{ guild_id: string; pending_cap_seconds: number }
		>({
			query: ({ guild_id, pending_cap_seconds }) => ({
				url: apiUrl(API_ROUTES.guildVoiceSettings, { guild_id }),
				method: "PUT",
				headers: JSON_BODY_HEADERS,
				body: { pending_cap_seconds },
			}),
			invalidatesTags: guildScoped("GuildVoiceSettings"),
		}),
		deleteGuildVoiceSettings: builder.mutation<GuildVoiceSettings, string>({
			query: (guild_id) => ({
				url: apiUrl(API_ROUTES.guildVoiceSettings, { guild_id }),
				method: "DELETE",
			}),
			invalidatesTags: guildScoped("GuildVoiceSettings"),
		}),
		getGuildRecordingPolicy: builder.query<GuildRecordingPolicy, string>({
			query: (guild_id) =>
				apiUrl(API_ROUTES.guildRecordingPolicy, { guild_id }),
			providesTags: guildScoped("GuildRecordingPolicy"),
		}),
		setGuildRecordingPolicy: builder.mutation<
			GuildRecordingPolicy,
			{
				guild_id: string;
				retention_days: number | null;
				excluded_channel_ids: string[];
			}
		>({
			query: ({ guild_id, retention_days, excluded_channel_ids }) => ({
				url: apiUrl(API_ROUTES.guildRecordingPolicy, { guild_id }),
				method: "PUT",
				body: { retention_days, excluded_channel_ids },
			}),
			invalidatesTags: guildScoped("GuildRecordingPolicy"),
		}),
		getRecordingOptOut: builder.query<RecordingOptOut, string>({
			query: (guild_id) => apiUrl(API_ROUTES.recordingOptOut, { guild_id }),
			providesTags: guildScoped("RecordingOptOut"),
		}),
		setRecordingOptOut: builder.mutation<
			RecordingOptOut,
			{ guild_id: string; opted_out: boolean }
		>({
			query: ({ guild_id, opted_out }) => ({
				url: apiUrl(API_ROUTES.recordingOptOut, { guild_id }),
				method: "PUT",
				body: { opted_out },
			}),
			invalidatesTags: guildScoped("RecordingOptOut"),
		}),
		getRecordingPrivacy: builder.query<RecordingPrivacy, string>({
			query: (guild_id) => apiUrl(API_ROUTES.recordingPrivacy, { guild_id }),
			providesTags: guildScoped("RecordingPrivacy"),
		}),
		setRecordingPrivacy: builder.mutation<
			RecordingPrivacy,
			{ guild_id: string; private: boolean }
		>({
			query: ({ guild_id, private: keepPrivate }) => ({
				url: apiUrl(API_ROUTES.recordingPrivacy, { guild_id }),
				method: "PUT",
				body: { private: keepPrivate },
			}),
			invalidatesTags: guildScoped("RecordingPrivacy"),
		}),
		deleteRecording: builder.mutation<
			RecordingDeletionStatus,
			{ guild_id: string; recording_session_id: string }
		>({
			query: ({ guild_id, recording_session_id }) => ({
				url: apiUrl(API_ROUTES.deleteRecording, {
					guild_id,
					recording_session_id,
				}),
				method: "DELETE",
			}),
			// "Clips" stays untagged: getClips provides the bare tag, which an
			// id-scoped invalidation would not match. A removal frees its keeps.
			invalidatesTags: (_r, _e, { guild_id }) => [
				...guildScoped("Recordings", "RemovedRecordings", "Keeping")(
					_r,
					_e,
					guild_id,
				),
				"Clips" as const,
			],
		}),
		/** The speaker erases their own session: hidden from everyone else now. */
		eraseSession: builder.mutation<
			ErasureStatus,
			{ guild_id: string; recording_session_id: string }
		>({
			query: ({ guild_id, recording_session_id }) => ({
				url: apiUrl(API_ROUTES.sessionErasure, {
					guild_id,
					recording_session_id,
				}),
				method: "POST",
			}),
			invalidatesTags: (_r, _e, { guild_id, recording_session_id }) => [
				{ type: "Recordings" as const, id: guild_id },
				{ type: "Stamps" as const, id: guild_id },
				{ type: "Keeping" as const, id: guild_id },
				{ type: "Session" as const, id: recording_session_id },
				"Clips" as const,
			],
		}),
		/** The speaker erases a clip of their voice, and what was made of it. */
		eraseClip: builder.mutation<
			ErasureStatus,
			{ guild_id: string; clip_id: string }
		>({
			query: ({ guild_id, clip_id }) => ({
				url: apiUrl(API_ROUTES.clipErasure, { guild_id, clip_id }),
				method: "POST",
			}),
			invalidatesTags: ["Clips"],
		}),
		/** Undoes an erasure while its grace period lasts. */
		undoErasure: builder.mutation<
			ErasureStatus,
			{ guild_id: string; erasure_id: string }
		>({
			query: ({ guild_id, erasure_id }) => ({
				url: apiUrl(API_ROUTES.erasure, { guild_id, erasure_id }),
				method: "DELETE",
			}),
			invalidatesTags: (result, _e, { guild_id }) => [
				{ type: "Recordings" as const, id: guild_id },
				{ type: "Stamps" as const, id: guild_id },
				result?.recording_session_id
					? { type: "Session" as const, id: result.recording_session_id }
					: ("Session" as const),
				"Clips" as const,
			],
		}),
		getRecordingDeletion: builder.query<
			RecordingDeletionStatus,
			{ guild_id: string; job_id: string }
		>({
			query: ({ guild_id, job_id }) =>
				apiUrl(API_ROUTES.recordingDeletion, { guild_id, job_id }),
			providesTags: (_result, _error, { job_id }) => jobTags(job_id),
		}),
		getGuildRoles: builder.query<GuildRole[], string>({
			query: (guild_id) => apiUrl(API_ROUTES.guildRoles, { guild_id }),
			providesTags: guildScoped("GuildRoles"),
		}),
		getRoleMembers: builder.query<
			RoleMember[],
			{ guild_id: string; role_id: string }
		>({
			query: ({ guild_id, role_id }) =>
				apiUrl(API_ROUTES.roleMembers, { guild_id, role_id }),
			providesTags: guildScoped("GuildRoles"),
		}),
		/** Manager-only member search; `q` matches names or the start of an id. */
		searchGuildMembers: builder.query<
			MemberPage,
			{ guild_id: string; q: string; offset?: number }
		>({
			query: ({ guild_id, q, offset }) => {
				const params = new URLSearchParams();
				if (q) params.set("q", q);
				if (offset) params.set("offset", String(offset));
				const search = params.toString();
				return `${apiUrl(API_ROUTES.guildMembers, { guild_id })}${search ? `?${search}` : ""}`;
			},
			providesTags: guildScoped("GuildMembers"),
		}),
		/** Who is in the voice and stage channels the viewer can view. */
		getVoicePresence: builder.query<
			VoicePresence,
			{ guild_id: string; as_role?: string }
		>({
			query: ({ guild_id, as_role }) =>
				withAsRole(apiUrl(API_ROUTES.voicePresence, { guild_id }), as_role),
			providesTags: guildScoped("VoicePresence"),
		}),
		getPermissionRules: builder.query<PermissionRules, string>({
			query: (guild_id) => apiUrl(API_ROUTES.permissionRules, { guild_id }),
			providesTags: guildScoped("PermissionRules"),
		}),
		/**
		 * Sets one rule; allowing and denying nothing deletes it. The cached
		 * rules take the change as it is sent, so an editor that drops its draft
		 * once saved shows the saved rule, not the old one until the refetch.
		 */
		setPermissionRule: builder.mutation<
			void,
			RuleChange & { guild_id: string }
		>({
			query: ({ guild_id, ...rule }) => ({
				url: apiUrl(API_ROUTES.permissionRules, { guild_id }),
				method: "PUT",
				body: rule,
			}),
			onQueryStarted({ guild_id, ...change }, api) {
				const channel = change.channel_id ?? null;
				const user = apiSlice.endpoints.getAuthDetails.select()(api.getState())
					.data?.user;
				const patch = api.dispatch(
					apiSlice.util.updateQueryData(
						"getPermissionRules",
						guild_id,
						({ rules }) =>
							writeEntry(
								rules,
								(rule) =>
									(rule.channel_id ?? null) === channel &&
									sameSubject(rule.subject, change.subject),
								(rule) =>
									change.allow.length === 0 && change.deny.length === 0
										? null
										: {
												name: null,
												reaches_nobody: false,
												...rule,
												...change,
												updated_by: user?.user_id ?? "",
												updated_at: new Date().toISOString(),
											},
							),
					),
				);
				api.queryFulfilled.catch(patch.undo);
			},
			invalidatesTags: guildScoped("PermissionRules", "PermissionExplanation"),
		}),
		/** What a member (`user_id`) or a role (`role_id`) holds, and why. */
		explainAccess: builder.query<
			AccessExplanation,
			{ guild_id: string; user_id?: string; role_id?: string }
		>({
			query: ({ guild_id, user_id, role_id }) => {
				const params = new URLSearchParams();
				if (user_id) params.set("user_id", user_id);
				if (role_id) params.set("role_id", role_id);
				return `${apiUrl(API_ROUTES.permissionExplanation, { guild_id })}?${params}`;
			},
			providesTags: guildScoped("PermissionExplanation"),
		}),
		/** The guild's cap, the viewer's allowance, and what is kept. */
		getKeeping: builder.query<Keeping, string>({
			query: (guild_id) => apiUrl(API_ROUTES.keptRecordings, { guild_id }),
			providesTags: guildScoped("Keeping"),
		}),
		/** Removed recordings the viewer may keep or restore, a page at a time. */
		getRemovedRecordings: builder.infiniteQuery<
			RemovedRecordings,
			string,
			string | null
		>({
			infiniteQueryOptions: {
				initialPageParam: null,
				getNextPageParam: (page) => page.next ?? undefined,
			},
			query: ({ queryArg: guild_id, pageParam }) => {
				const url = apiUrl(API_ROUTES.removedRecordings, { guild_id });
				return pageParam
					? `${url}?before=${encodeURIComponent(pageParam)}`
					: url;
			},
			providesTags: guildScoped("RemovedRecordings"),
		}),
		/** Keeps a recording, bringing it back if retention removed it. */
		keepRecording: builder.mutation<
			void,
			{ guild_id: string; recording_session_id: string }
		>({
			query: ({ guild_id, recording_session_id }) => ({
				url: apiUrl(API_ROUTES.keptRecording, {
					guild_id,
					recording_session_id,
				}),
				method: "PUT",
			}),
			invalidatesTags: (_r, _e, { guild_id }) => [
				...guildScoped(
					"Keeping",
					"KeepAllowances",
					"RemovedRecordings",
					"Recordings",
				)(_r, _e, guild_id),
				"Clips" as const,
			],
		}),
		/** The viewer stops keeping a recording. */
		releaseRecording: builder.mutation<
			void,
			{ guild_id: string; recording_session_id: string }
		>({
			query: ({ guild_id, recording_session_id }) => ({
				url: apiUrl(API_ROUTES.keptRecording, {
					guild_id,
					recording_session_id,
				}),
				method: "DELETE",
			}),
			invalidatesTags: guildScoped("Keeping", "KeepAllowances"),
		}),
		/** A manager stops everyone keeping a recording. */
		releaseRecordingForEveryone: builder.mutation<
			void,
			{ guild_id: string; recording_session_id: string }
		>({
			query: ({ guild_id, recording_session_id }) => ({
				url: apiUrl(API_ROUTES.keptRecordingForEveryone, {
					guild_id,
					recording_session_id,
				}),
				method: "DELETE",
			}),
			invalidatesTags: guildScoped("Keeping", "KeepAllowances"),
		}),
		/** A manager brings back a removal within the retention period. */
		restoreRecording: builder.mutation<
			void,
			{ guild_id: string; recording_session_id: string }
		>({
			query: ({ guild_id, recording_session_id }) => ({
				url: apiUrl(API_ROUTES.restoreRecording, {
					guild_id,
					recording_session_id,
				}),
				method: "POST",
			}),
			invalidatesTags: (_r, _e, { guild_id }) => [
				...guildScoped("RemovedRecordings", "Recordings")(_r, _e, guild_id),
				"Clips" as const,
			],
		}),
		getKeepAllowances: builder.query<KeepAllowances, string>({
			query: (guild_id) => apiUrl(API_ROUTES.keepAllowances, { guild_id }),
			providesTags: guildScoped("KeepAllowances"),
		}),
		/** Sets one allowance; null removes it. Cached as sent, like a rule. */
		setKeepAllowance: builder.mutation<
			void,
			KeepAllowanceChange & { guild_id: string }
		>({
			query: ({ guild_id, ...change }) => ({
				url: apiUrl(API_ROUTES.keepAllowances, { guild_id }),
				method: "PUT",
				body: change,
			}),
			onQueryStarted({ guild_id, subject, allowance }, api) {
				const patch = api.dispatch(
					apiSlice.util.updateQueryData(
						"getKeepAllowances",
						guild_id,
						({ allowances }) =>
							writeEntry(
								allowances,
								(entry) => sameSubject(entry.subject, subject),
								(entry) =>
									allowance == null
										? null
										: {
												name: null,
												reaches_nobody: false,
												...entry,
												subject,
												allowance,
											},
							),
					),
				);
				api.queryFulfilled.catch(patch.undo);
			},
			invalidatesTags: guildScoped("KeepAllowances", "Keeping"),
		}),
		getOperatorGuilds: builder.query<OperatorGuild[], void>({
			query: () => apiUrl(API_ROUTES.operatorGuilds),
			providesTags: ["OperatorGuilds"],
		}),
		/** How many recordings a guild may keep: operators only. Cached as sent. */
		setGuildLimits: builder.mutation<
			void,
			{ guild_id: string; kept_recordings: number }
		>({
			query: ({ guild_id, kept_recordings }) => ({
				url: apiUrl(API_ROUTES.guildLimits, { guild_id }),
				method: "PUT",
				body: { kept_recordings },
			}),
			onQueryStarted({ guild_id, kept_recordings }, api) {
				const patch = api.dispatch(
					apiSlice.util.updateQueryData(
						"getOperatorGuilds",
						undefined,
						(guilds) => {
							const guild = guilds.find((g) => g.guild_id === guild_id);
							if (guild) guild.kept_recordings_cap = kept_recordings;
						},
					),
				);
				api.queryFulfilled.catch(patch.undo);
			},
			invalidatesTags: (_r, _e, { guild_id }) => [
				"OperatorGuilds" as const,
				...guildScoped("Keeping", "KeepAllowances")(_r, _e, guild_id),
			],
		}),
		getRoleView: builder.query<RoleView, { guild_id: string; role_id: string }>(
			{
				query: ({ guild_id, role_id }) =>
					apiUrl(API_ROUTES.roleChannels, { guild_id, role_id }),
				// A role's decisions, like the explainer's: a saved rule or a
				// roster change must not leave a cached preview behind.
				providesTags: guildScoped("PermissionExplanation"),
			},
		),
		// Combine all 3 requests into a single query to emulate the existing Promise.all behavior
		getAuthDetails: builder.query<AuthDetails, void>({
			// Invalidated when a refresh fails, so the shell drops back to the
			// login screen the moment the session expires.
			providesTags: ["Auth"],
			async queryFn(_arg, _queryApi, _extraOptions, fetchWithBQ) {
				const [userResult, guildsResult] = await Promise.all([
					fetchWithBQ(apiUrl(API_ROUTES.currentUser)),
					fetchWithBQ(apiUrl(API_ROUTES.currentUserGuilds)),
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
	usePlayClipMutation,
	useRemoveSilenceMutation,
	useCheckSilenceFileQuery,
	useRefreshMutation,
	useEndSessionMutation,
	useGetSessionsQuery,
	useLogoutEverywhereMutation,
	useLogoutMutation,
	useCreateClipMutation,
	useGetLiveStateQuery,
	useGetRecordingEventsQuery,
	useGetWaveformQuery,
	useGetWaveformByUrlQuery,
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
	useGetGuildRecordingPolicyQuery,
	useSetGuildRecordingPolicyMutation,
	useGetRecordingOptOutQuery,
	useSetRecordingOptOutMutation,
	useGetRecordingPrivacyQuery,
	useSetRecordingPrivacyMutation,
	useDeleteRecordingMutation,
	useGetRecordingDeletionQuery,
	useEraseSessionMutation,
	useEraseClipMutation,
	useUndoErasureMutation,
	useGetGuildRolesQuery,
	useGetRoleMembersQuery,
	useSearchGuildMembersQuery,
	useGetVoicePresenceQuery,
	useGetRoleViewQuery,
	useGetPermissionRulesQuery,
	useSetPermissionRuleMutation,
	useExplainAccessQuery,
	useGetKeepingQuery,
	useGetRemovedRecordingsInfiniteQuery,
	useKeepRecordingMutation,
	useReleaseRecordingMutation,
	useReleaseRecordingForEveryoneMutation,
	useRestoreRecordingMutation,
	useGetKeepAllowancesQuery,
	useSetKeepAllowanceMutation,
	useGetOperatorGuildsQuery,
	useSetGuildLimitsMutation,
} = apiSlice;
