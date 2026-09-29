import { apiSlice } from "../app/apiSlice";
import { store } from "../store";
import { guildRouteLoader, routeQueryOptions } from "./guildRouteLoader";

/** Runs alongside the route import, before any audio components mount. */
export const audioRouteLoader = guildRouteLoader((args) => {
	void store.dispatch(
		apiSlice.endpoints.getCurrentGuildDirs.initiate(args, routeQueryOptions),
	);
	void store.dispatch(
		apiSlice.endpoints.getLiveStems.initiate(args, routeQueryOptions),
	);
});
