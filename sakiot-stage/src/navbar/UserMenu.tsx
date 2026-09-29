import { Check } from "lucide-react";
import { useState } from "react";
import { problemFromError } from "../app/apiError";
import {
	type User,
	useGetRecordingOptOutQuery,
	useLogoutMutation,
	useSetRecordingOptOutMutation,
} from "../app/apiSlice";
import { setCsrfToken } from "../app/authedFetch";
import type { UserGuilds } from "../Constants";
import { discordAvatarUrl } from "../shared/discordAvatar";
import {
	Avatar,
	cn,
	IconButton,
	Menu,
	MenuItem,
	MenuSection,
	MenuTrigger,
	Notice,
	Popover,
} from "../shared/ui";

const RECORD_ME = "record-me";

export function UserMenu(props: { user: User; guild: UserGuilds | null }) {
	const { user, guild } = props;
	const [logout] = useLogoutMutation();
	// Recording is per server and on by default; members opt out here or with
	// `/recording opt-out` in Discord.
	const { data: optOut, isError: optOutUnavailable } =
		useGetRecordingOptOutQuery(guild?.id ?? "", { skip: !guild });
	const [setOptOut] = useSetRecordingOptOutMutation();
	const [menuError, setMenuError] = useState<string | null>(null);

	const handleAction = async (key: string) => {
		if (key !== "Logout") return;
		setMenuError(null);
		try {
			await logout().unwrap();
		} catch (error) {
			// 401 means the session was already gone, which is a successful
			// logout. Anything else (network, 5xx) may leave the server session
			// valid, so say so instead of reloading back into it.
			const status = (error as { status?: number } | null)?.status;
			if (status !== 401) {
				setMenuError("Could not log out. Check your connection and try again.");
				return;
			}
		}
		setCsrfToken(null);
		window.location.assign("/");
	};

	const setRecorded = async (recorded: boolean) => {
		if (!guild) return;
		setMenuError(null);
		try {
			await setOptOut({ guild_id: guild.id, opted_out: !recorded }).unwrap();
		} catch (error) {
			setMenuError(
				`Could not change whether you are recorded. ${problemFromError(error).message}`,
			);
		}
	};

	return (
		<>
			<MenuTrigger>
				<IconButton aria-label="Open user menu" size="md" className="p-0">
					<Avatar
						alt={`${user.username} avatar`}
						src={discordAvatarUrl(user) ?? undefined}
					>
						{user.username.slice(0, 1).toUpperCase()}
					</Avatar>
				</IconButton>
				<Popover placement="bottom end" className="z-[70]">
					{/* Fixed width: a content-sized menu this close to the viewport
					    edge settled over two layout passes, which the browser
					    reported as a ResizeObserver loop. */}
					<Menu
						className="w-72"
						onAction={(key) => void handleAction(String(key))}
					>
						{guild && !optOutUnavailable ? (
							<MenuSection
								aria-label="Recording"
								// Stay open so the check mark visibly follows the change.
								shouldCloseOnSelect={false}
								selectionMode="multiple"
								selectedKeys={optOut?.opted_out === false ? [RECORD_ME] : []}
								onSelectionChange={(keys) =>
									void setRecorded(keys === "all" || keys.has(RECORD_ME))
								}
							>
								{/* Present while its state loads, so the open menu never
								    changes size underneath the pointer. */}
								<MenuItem
									id={RECORD_ME}
									isDisabled={!optOut}
									textValue={`Record my voice in ${guild.name}`}
								>
									{({ isSelected }) => (
										<span className="flex items-center gap-2">
											<Check
												aria-hidden="true"
												size={16}
												className={cn("shrink-0", !isSelected && "invisible")}
											/>
											Record my voice in {guild.name}
										</span>
									)}
								</MenuItem>
							</MenuSection>
						) : null}
						<MenuSection aria-label="Account">
							<MenuItem id="Logout">Log out</MenuItem>
						</MenuSection>
					</Menu>
				</Popover>
			</MenuTrigger>
			{menuError && (
				<div className="fixed bottom-4 right-4 z-[70]">
					<Notice tone={"error"} announce="alert">
						{menuError}
					</Notice>
				</div>
			)}
		</>
	);
}
