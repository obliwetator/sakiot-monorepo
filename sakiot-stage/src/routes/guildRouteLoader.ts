import type {
	LoaderFunctionArgs,
	ShouldRevalidateFunctionArgs,
} from "react-router-dom";
import { apiSlice } from "../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../app/authedFetch";
import { store } from "../store";

type GuildQueryArgs = { guild_id: string; as_role?: string };

export const routeQueryOptions = {
	subscribe: false,
	forceRefetch: true,
} as const;

/** Start authorized queries alongside the route import, before mounting UI. */
export function guildRouteLoader(preload: (args: GuildQueryArgs) => void) {
	return async ({ params, request }: LoaderFunctionArgs) => {
		if (!params.guild_id || !hasLoggedInCookie() || request.signal.aborted)
			return null;

		// Router initialization precedes AuthenticatedApp. Join its auth probe
		// (or start it); render-time access boundaries cannot guard loaders.
		await store.dispatch(
			apiSlice.endpoints.getAuthDetails.initiate(undefined, {
				subscribe: false,
			}),
		);
		const auth = apiSlice.endpoints.getAuthDetails.select()(store.getState());
		if (
			request.signal.aborted ||
			!auth.isSuccess ||
			!auth.data?.user ||
			!auth.data.guilds?.some((guild) => guild.id === params.guild_id)
		)
			return null;

		const asRole = new URL(request.url).searchParams.get("as_role");
		preload({
			guild_id: params.guild_id,
			...(asRole ? { as_role: asRole } : {}),
		});
		// The page subscribes to these cache entries and renders loading/errors.
		// Response bodies must not hold up navigation or create permanent subscriptions.
		return null;
	};
}

export function shouldRevalidateGuildData({
	currentParams,
	nextParams,
	currentUrl,
	nextUrl,
	defaultShouldRevalidate,
}: ShouldRevalidateFunctionArgs) {
	if (
		currentParams.guild_id !== nextParams.guild_id ||
		(currentUrl.searchParams.get("as_role") || "") !==
			(nextUrl.searchParams.get("as_role") || "")
	)
		return true;
	// Keep explicit revalidation and same-URL refreshes. Selecting a recording,
	// clip, or seek time within the same guild does not reload the whole list.
	return currentUrl.href === nextUrl.href && defaultShouldRevalidate;
}
