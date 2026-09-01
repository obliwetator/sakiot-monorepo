import type { ChangeEvent } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import type { UserGuilds } from "../../Constants";
import { Box, FormControl, InputLabel, MenuItem, Select } from "../ui";

export function BasicSelect(props: {
	guildSelected: UserGuilds | null;
	setGuildSelected: (guild: UserGuilds | null) => void;
	userGuilds: UserGuilds[] | null;
}) {
	const navigate = useNavigate();
	const location = useLocation();

	const handleChange = (event: ChangeEvent<HTMLSelectElement>) => {
		const newGuild = props.userGuilds?.find(
			(item) => item.name === event.target.value,
		);
		if (!newGuild) return;

		props.setGuildSelected(newGuild);

		if (
			props.guildSelected &&
			location.pathname.includes(props.guildSelected.id)
		) {
			const result = location.pathname.split(props.guildSelected.id);
			navigate(result[0] + newGuild.id);
		}
	};

	const guilds = props.userGuilds?.map((value) => (
		<MenuItem key={value.id} value={value.name}>
			{value.name}
		</MenuItem>
	));

	return (
		<Box sx={{ minWidth: 121 }}>
			<FormControl fullWidth>
				<InputLabel id="demo-simple-select-label" className="bg-header">
					Server
				</InputLabel>
				<Select
					labelId="demo-simple-select-label"
					id="demo-simple-select"
					label="Server"
					onChange={handleChange}
					value={props.guildSelected ? props.guildSelected.name : ""}
				>
					{guilds}
				</Select>
			</FormControl>
		</Box>
	);
}
