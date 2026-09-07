import { useLocation, useNavigate } from "react-router-dom";
import type { UserGuilds } from "../Constants";
import { Select, SelectItem } from "./ui";

export function GuildSelect(props: {
	guildSelected: UserGuilds | null;
	setGuildSelected: (guild: UserGuilds | null) => void;
	userGuilds: UserGuilds[] | null;
}) {
	const navigate = useNavigate();
	const location = useLocation();
	return (
		<Select
			label="Server"
			labelPlacement="floating"
			className="min-w-[121px]"
			selectedKey={props.guildSelected?.id ?? null}
			onSelectionChange={(key) => {
				const guild = props.userGuilds?.find((item) => item.id === key);
				if (!guild) return;
				props.setGuildSelected(guild);
				if (
					props.guildSelected &&
					location.pathname.includes(props.guildSelected.id)
				) {
					const [prefix] = location.pathname.split(props.guildSelected.id);
					navigate(prefix + guild.id);
				}
			}}
		>
			{props.userGuilds?.map((guild) => (
				<SelectItem key={guild.id} id={guild.id}>
					{guild.name}
				</SelectItem>
			))}
		</Select>
	);
}
