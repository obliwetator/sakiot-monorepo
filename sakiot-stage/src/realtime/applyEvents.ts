import type { components } from "../api/openapi";
import { type ApiTag, apiSlice } from "../app/apiSlice";
import type { AppDispatch, RootState } from "../store";
import { patchSession } from "./patchTree";

type ServerMessage = components["schemas"]["ServerMessage"];
export type ChangedMessage = Extract<ServerMessage, { type: "changed" }>;

export interface ApplyContext {
	dispatch: AppDispatch;
	getState: () => RootState;
	/**
	 * Changes whenever the scope or account changes. A targeted fetch that
	 * finishes under a newer generation is dropped: it may describe what the
	 * previous viewer, guild or role preview was allowed to see.
	 */
	generation: () => number;
}

/** Every guild-scoped tag a subscribed page may hold. */
const GUILD_TAGS: ApiTag[] = [
	"Recordings",
	"LiveStems",
	"LiveState",
	"Clips",
	"Stamps",
	"RecordingOptOut",
	"GuildVoiceSettings",
	"GuildRecordingPolicy",
	"GuildCooldown",
	"UserOverrides",
	"GuildRoles",
	"GuildMembers",
	"VoicePresence",
];

function invalidate(ctx: ApplyContext, guildId: string, types: ApiTag[]) {
	ctx.dispatch(
		apiSlice.util.invalidateTags(types.map((type) => ({ type, id: guildId }))),
	);
}

/**
 * Refetch everything the guild's pages show (after `subscribed`, a resync,
 * or a permission change). RTK refetches what is mounted and marks the rest
 * stale for its next use.
 */
export function reconcileGuild(ctx: ApplyContext, guildId: string): void {
	invalidate(ctx, guildId, GUILD_TAGS);
	// Job events may have been missed too; only queries still waiting on a
	// job carry its tag.
	ctx.dispatch(apiSlice.util.invalidateTags(["Session", "Job"]));
}

const inFlight = new Map<string, { again: boolean }>();

/**
 * Re-reads one session for one cached tree (one `as_role` variant) and
 * patches it in. A change arriving while the fetch is in flight triggers one
 * trailing fetch, so the last event always wins.
 */
async function refreshTreeEntry(
	ctx: ApplyContext,
	arg: { guild_id: string; as_role?: string },
	sessionId: string,
): Promise<void> {
	const key = `${arg.guild_id}|${arg.as_role ?? ""}|${sessionId}`;
	const running = inFlight.get(key);
	if (running) {
		running.again = true;
		return;
	}
	const state = { again: false };
	inFlight.set(key, state);
	try {
		do {
			state.again = false;
			const generation = ctx.generation();
			const result = await ctx.dispatch(
				apiSlice.endpoints.getSessionListingEntry.initiate(
					{ ...arg, recording_session_id: sessionId },
					{ subscribe: false, forceRefetch: true },
				),
			);
			if (generation !== ctx.generation()) return;
			if (result.error) {
				// Could not read the session: refetch the whole tree instead.
				invalidate(ctx, arg.guild_id, ["Recordings"]);
				return;
			}
			ctx.dispatch(
				apiSlice.util.updateQueryData("getCurrentGuildDirs", arg, (tree) => {
					patchSession(tree, sessionId, result.data ?? null);
				}),
			);
		} while (state.again);
	} finally {
		inFlight.delete(key);
	}
}

function refreshSessions(
	ctx: ApplyContext,
	guildId: string,
	sessionIds: string[],
): void {
	ctx.dispatch(
		apiSlice.util.invalidateTags(
			sessionIds.map((id) => ({ type: "Session" as const, id })),
		),
	);
	invalidate(ctx, guildId, ["LiveStems", "LiveState"]);

	const state = ctx.getState();
	const args = apiSlice.util
		.selectCachedArgsForQuery(state, "getCurrentGuildDirs")
		.filter((arg) => arg.guild_id === guildId);
	for (const arg of args) {
		const cached = apiSlice.endpoints.getCurrentGuildDirs.select(arg)(state);
		if (cached.status !== "fulfilled") {
			// Still loading: RTK refetches it once the pending load finishes,
			// so the change is not lost to the load's older snapshot.
			invalidate(ctx, guildId, ["Recordings"]);
			continue;
		}
		for (const id of sessionIds) void refreshTreeEntry(ctx, arg, id);
	}
}

export function applyChanged(ctx: ApplyContext, message: ChangedMessage): void {
	const guildId = message.guild_id;
	switch (message.resource) {
		case "recordings":
			if (message.ids && message.ids.length > 0) {
				refreshSessions(ctx, guildId, message.ids);
			} else {
				// Visibility may have changed: refresh everything derived from
				// the guild's recordings, without being told which session.
				invalidate(ctx, guildId, [
					"Recordings",
					"LiveStems",
					"LiveState",
					"Clips",
					"Stamps",
				]);
				ctx.dispatch(apiSlice.util.invalidateTags(["Session"]));
			}
			return;
		case "clips":
			invalidate(ctx, guildId, ["Clips"]);
			return;
		case "stamps":
			invalidate(ctx, guildId, ["Stamps"]);
			return;
		case "recording_opt_out":
			invalidate(ctx, guildId, ["RecordingOptOut"]);
			return;
		case "voice_settings":
			invalidate(ctx, guildId, ["GuildVoiceSettings"]);
			return;
		case "recording_policy":
			invalidate(ctx, guildId, ["GuildRecordingPolicy"]);
			return;
		case "cooldowns":
			invalidate(ctx, guildId, ["GuildCooldown", "UserOverrides"]);
			return;
		case "presence":
			invalidate(ctx, guildId, ["VoicePresence"]);
			return;
		case "members":
			// Role pages count and list members from the roster.
			invalidate(ctx, guildId, ["GuildMembers", "GuildRoles"]);
			return;
		case "jobs":
			// Only the queries still waiting on these jobs refetch.
			ctx.dispatch(
				apiSlice.util.invalidateTags(
					message.ids
						? message.ids.map((id) => ({ type: "Job" as const, id }))
						: ["Job"],
				),
			);
			return;
	}
}
