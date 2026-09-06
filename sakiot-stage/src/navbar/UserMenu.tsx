import {
	Avatar,
	IconButton,
	Menu,
	MenuItem,
	MenuTrigger,
	Popover,
} from "../shared/ui";
import { settings } from "./constants";

export function UserMenu() {
	return (
		<MenuTrigger>
			<IconButton aria-label="Open settings" size="md" className="p-0">
				<Avatar alt="" src="/pepega.png" />
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
