import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { Notice } from "../../shared/ui";

/** Tells a former member why a guild shows only their own recordings. */
export function FormerMemberBanner({ guildId }: { guildId: string }) {
	const { data } = useGetAuthDetailsQuery();
	const guild = data?.guilds?.find((guild) => guild.id === guildId);
	if (!guild?.former) return null;
	return (
		<div className="mb-4">
			<Notice tone={"info"} announce="status">
				You are no longer a member of <strong>{guild.name}</strong>, so only
				recordings of your own voice are shown.
			</Notice>
		</div>
	);
}
