import type { User } from "../app/apiSlice";
import { discordAvatarUrl } from "../shared/discordAvatar";
import {
	Avatar,
	IconButton,
	Menu,
	MenuItem,
	MenuTrigger,
	Popover,
} from "../shared/ui";
import { settings } from "./constants";

export function UserMenu(props: { user: User | null }) {
	const { user } = props;
	return (
		<MenuTrigger>
			<IconButton aria-label="Open settings" size="md" className="p-0">
				<Avatar
					alt={user ? `${user.username} avatar` : ""}
					src={discordAvatarUrl(user) ?? undefined}
				>
					{user ? user.username.slice(0, 1).toUpperCase() : null}
				</Avatar>
			</IconButton>
			<Popover placement="bottom end" className="z-[70]">
				<Menu>
					{settings.map((setting) => (
						<MenuItem key={setting} id={setting}>
							{setting}
						</MenuItem>
					))}
				</Menu>
			</Popover>
		</MenuTrigger>
	);
}
