import { Outlet, useParams } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../app/apiSlice";
import { RouteState } from "../routes/RouteState";

/** Guard route-level guild access before rendering pages that request media. */
export function GuildAccessBoundary() {
	const { guild_id: guildId } = useParams();
	const { data, isLoading } = useGetAuthDetailsQuery();
	if (isLoading || !data) {
		return (
			<p className="p-6" role="status">
				Loading server…
			</p>
		);
	}
	if (!guildId || !(data.guilds ?? []).some((guild) => guild.id === guildId)) {
		return <RouteState kind="forbidden" />;
	}
	return <Outlet />;
}
