import type {
	LoaderFunctionArgs,
	ShouldRevalidateFunctionArgs,
} from "react-router-dom";
import { apiSlice } from "../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../app/authedFetch";
import { store } from "../store";

/** Runs alongside the route import, before any audio components mount. */
export async function audioRouteLoader({
	params,
	request,
}: LoaderFunctionArgs) {
	if (!params.guild_id || !hasLoggedInCookie() || request.signal.aborted)
		return null;

	// The router initializes before AuthenticatedApp. Join its RTK Query auth
	// probe (or start it), rather than relying on a render-time route guard.
	await store.dispatch(
		apiSlice.endpoints.getAuthDetails.initiate(undefined, { subscribe: false }),
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
	const args = {
		guild_id: params.guild_id,
		...(asRole ? { as_role: asRole } : {}),
	};
	// Warm the same cache the mounted tree subscribes to. Do not block route
	// rendering on the response bodies or leave permanent loader subscriptions.
	void store.dispatch(
		apiSlice.endpoints.getCurrentGuildDirs.initiate(args, {
			subscribe: false,
			forceRefetch: true,
		}),
	);
	void store.dispatch(
		apiSlice.endpoints.getLiveStems.initiate(args, {
			subscribe: false,
			forceRefetch: true,
		}),
	);
	return null;
}

export function shouldRevalidateAudio({
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
	// Keep explicit revalidation (including login) and same-URL refreshes, but
	// selecting a recording or changing its seek time does not reload the tree.
	return currentUrl.href === nextUrl.href && defaultShouldRevalidate;
}
