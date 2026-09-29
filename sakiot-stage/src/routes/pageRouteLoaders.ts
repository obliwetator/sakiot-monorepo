import { apiSlice } from "../app/apiSlice";
import { store } from "../store";
import { guildRouteLoader, routeQueryOptions } from "./guildRouteLoader";

export const clipsRouteLoader = guildRouteLoader((args) => {
	void store.dispatch(
		apiSlice.endpoints.getClips.initiate(args, routeQueryOptions),
	);
});

export const stampsRouteLoader = guildRouteLoader((args) => {
	void store.dispatch(
		apiSlice.endpoints.getStamps.initiate(args, routeQueryOptions),
	);
});

export const cooldownsRouteLoader = guildRouteLoader(({ guild_id }) => {
	void store.dispatch(
		apiSlice.endpoints.getGuildCooldown.initiate(guild_id, routeQueryOptions),
	);
	void store.dispatch(
		apiSlice.endpoints.listUserOverrides.initiate(guild_id, routeQueryOptions),
	);
});

export const voiceSettingsRouteLoader = guildRouteLoader(({ guild_id }) => {
	void store.dispatch(
		apiSlice.endpoints.getGuildVoiceSettings.initiate(
			guild_id,
			routeQueryOptions,
		),
	);
	void store.dispatch(
		apiSlice.endpoints.getGuildRecordingPolicy.initiate(
			guild_id,
			routeQueryOptions,
		),
	);
});

export const membersRouteLoader = guildRouteLoader(({ guild_id }) => {
	void store.dispatch(
		apiSlice.endpoints.getGuildRoles.initiate(guild_id, routeQueryOptions),
	);
});
