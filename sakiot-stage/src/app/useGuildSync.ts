import { useEffect } from "react";
import { useDispatch } from "react-redux";
import { useMatch } from "react-router-dom";
import type { UserGuilds } from "../Constants";
import { setGuildSelected } from "../reducers/appSlice";
import { useAppSelector } from "./hooks";

function useGuildIdFromRoute(): string | undefined {
	const dashboard = useMatch("/dashboard/:guild_id/*");
	const stamps = useMatch("/stamps/:guild_id");
	return dashboard?.params.guild_id ?? stamps?.params.guild_id;
}

export function useGuildSync(userGuilds: UserGuilds[] | null) {
	const dispatch = useDispatch();
	const current = useAppSelector((s) => s.app.guildSelected);
	const routeGuildId = useGuildIdFromRoute();

	useEffect(() => {
		if (!routeGuildId || !userGuilds) return;
		const match = userGuilds.find((g) => g.id === routeGuildId);
		if (current?.id !== match?.id) dispatch(setGuildSelected(match ?? null));
	}, [routeGuildId, userGuilds, current?.id, dispatch]);
}
