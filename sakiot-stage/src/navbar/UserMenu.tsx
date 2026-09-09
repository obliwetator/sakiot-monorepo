import { useState } from "react";
import { type User, useLogoutMutation } from "../app/apiSlice";
import { setCsrfToken } from "../app/authedFetch";
import { discordAvatarUrl } from "../shared/discordAvatar";
import {
	Avatar,
	IconButton,
	Menu,
	MenuItem,
	MenuTrigger,
	Notice,
	Popover,
} from "../shared/ui";
import { settings } from "./constants";

export function UserMenu(props: { user: User | null }) {
	const { user } = props;
	const [logout] = useLogoutMutation();
	const [logoutError, setLogoutError] = useState<string | null>(null);

	const handleAction = async (key: string) => {
		if (key !== "Logout") return;
		setLogoutError(null);
		try {
			await logout().unwrap();
		} catch (error) {
			// 401 means the session was already gone, which is a successful
			// logout. Anything else (network, 5xx) may leave the server session
			// valid, so say so instead of reloading back into it.
			const status = (error as { status?: number } | null)?.status;
			if (status !== 401) {
				setLogoutError(
					"Could not log out. Check your connection and try again.",
				);
				return;
			}
		}
		setCsrfToken(null);
		window.location.assign("/");
	};

	return (
		<>
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
					<Menu onAction={(key) => void handleAction(String(key))}>
						{settings.map((setting) => (
							<MenuItem key={setting} id={setting}>
								{setting}
							</MenuItem>
						))}
					</Menu>
				</Popover>
			</MenuTrigger>
			{logoutError && (
				<div className="fixed bottom-4 right-4 z-[70]">
					<Notice tone={"error"} announce="alert">
						{logoutError}
					</Notice>
				</div>
			)}
		</>
	);
}
