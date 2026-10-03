import { useEffect } from "react";
import type { createBrowserRouter } from "react-router-dom";
import { apiSlice } from "../app/apiSlice";
import { store } from "../store";
import { RealtimeCoordinator, type Scope } from "./coordinator";

type Router = ReturnType<typeof createBrowserRouter>;

let coordinator: RealtimeCoordinator | null = null;

function getCoordinator(): RealtimeCoordinator {
	coordinator ??= new RealtimeCoordinator(store.dispatch, store.getState);
	return coordinator;
}

/** The guild and role preview the current route shows, if any. */
function scopeOf(state: Router["state"]): Scope | null {
	const guildId = state.matches.at(-1)?.params.guild_id;
	if (!guildId) return null;
	const asRole =
		new URLSearchParams(state.location.search).get("as_role") ?? undefined;
	return { guildId, ...(asRole ? { asRole } : {}) };
}

/**
 * Keeps this tab's realtime socket running while `userId` is logged in on a
 * server with realtime enabled, scoped to whatever guild the router shows.
 */
export function useRealtime(
	router: Router,
	userId: string | null | undefined,
	enabled: boolean,
): void {
	useEffect(() => {
		if (!userId || !enabled) return;
		const realtime = getCoordinator();
		realtime.setScope(scopeOf(router.state));
		const unsubscribe = router.subscribe((state) =>
			realtime.setScope(scopeOf(state)),
		);
		realtime.start();
		return () => {
			unsubscribe();
			realtime.stop();
		};
	}, [router, userId, enabled]);
}

/**
 * Another tab of this origin logged in or out (the CSRF token changed in
 * shared storage): re-check who is logged in here. The app shell then
 * reconciles, and an account change drops the previous account's data.
 */
export function useCrossTabLogin(): void {
	useEffect(() => {
		const onStorage = (event: StorageEvent) => {
			if (event.key === "sakiot.csrf") {
				store.dispatch(apiSlice.util.invalidateTags(["Auth"]));
			}
		};
		window.addEventListener("storage", onStorage);
		return () => window.removeEventListener("storage", onStorage);
	}, []);
}

/** Tags for everything private to the logged-in account. */
const PRIVATE_TAGS = [
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
	"Session",
] as const;

/** The account changed: refetch, and never show, the previous one's data. */
export function dropPreviousAccountData(): void {
	store.dispatch(apiSlice.util.invalidateTags([...PRIVATE_TAGS]));
}
