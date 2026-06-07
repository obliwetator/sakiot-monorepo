import { useEffect } from "react";
import { useDispatch } from "react-redux";
import { useMatch } from "react-router-dom";
import type { UserGuilds } from "../Constants";
import { setGuildSelected } from "../reducers/appSlice";
import { useAppSelector } from "./hooks";

function useGuildIdFromRoute(): string | undefined {
	const dashboard = useMatch("/dashboard/:guild_id/*");
	const metrics = useMatch("/metrics/:guild_id");
	const stamps = useMatch("/stamps/:guild_id");
	const root = useMatch("/:guild_id");
	const RESERVED = new Set(["dashboard", "metrics", "stamps"]);
	const rootId = root?.params.guild_id;
	return (
		dashboard?.params.guild_id ??
		metrics?.params.guild_id ??
		stamps?.params.guild_id ??
		(rootId && !RESERVED.has(rootId) ? rootId : undefined)
	);
}

export function useGuildSync(userGuilds: UserGuilds[] | null) {
	const dispatch = useDispatch();
	const current = useAppSelector((s) => s.app.guildSelected);
	const routeGuildId = useGuildIdFromRoute();

	useEffect(() => {
		if (!routeGuildId || !userGuilds) return;
		if (current?.id === routeGuildId) return;
		const match = userGuilds.find((g) => g.id === routeGuildId);
		if (match) dispatch(setGuildSelected(match));
	}, [routeGuildId, userGuilds, current?.id, dispatch]);
}
