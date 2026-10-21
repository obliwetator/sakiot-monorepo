import { Check } from "lucide-react";
import { type ReactNode, useState } from "react";
import { problemFromError } from "../app/apiError";
import {
	type User,
	useGetRecordingOptOutQuery,
	useGetRecordingPrivacyQuery,
	useLogoutEverywhereMutation,
	useLogoutMutation,
	useSetRecordingOptOutMutation,
	useSetRecordingPrivacyMutation,
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
import { SessionsDialog } from "./SessionsDialog";

const RECORD_ME = "record-me";
const ONLY_ME = "only-me";
const DEVICES = "devices";
const LOG_OUT = "log-out";
const LOG_OUT_EVERYWHERE = "log-out-everywhere";

export function UserMenu(props: { user: User; guild: UserGuilds | null }) {
	const { user, guild } = props;
	const [logout] = useLogoutMutation();
	const [logoutEverywhere] = useLogoutEverywhereMutation();
	// Recording is per server and on by default; members opt out here or with
	// `/recording opt-out` in Discord.
	const { data: optOut, isError: optOutUnavailable } =
		useGetRecordingOptOutQuery(guild?.id ?? "", { skip: !guild });
	const [setOptOut] = useSetRecordingOptOutMutation();
	// Who may hear their recordings: anyone the server lets listen, or only
	// themselves. Former members, who can no longer opt out, still choose.
	const { data: privacy, isError: privacyUnavailable } =
		useGetRecordingPrivacyQuery(guild?.id ?? "", { skip: !guild });
	const [setPrivacy] = useSetRecordingPrivacyMutation();
	const recorded = optOut?.opted_out === false;
	const onlyMe = privacy?.private === true;
	const [menuError, setMenuError] = useState<string | null>(null);
	const [devicesOpen, setDevicesOpen] = useState(false);

	const handleAction = async (key: string) => {
		if (key === DEVICES) {
			setDevicesOpen(true);
			return;
		}
		if (key !== LOG_OUT && key !== LOG_OUT_EVERYWHERE) return;
		setMenuError(null);
		try {
			// Everywhere ends this browser's session too.
			await (key === LOG_OUT ? logout() : logoutEverywhere()).unwrap();
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

	const setOnlyMe = async (onlyMe: boolean) => {
		if (!guild) return;
		setMenuError(null);
		try {
			await setPrivacy({ guild_id: guild.id, private: onlyMe }).unwrap();
		} catch (error) {
			setMenuError(
				`Could not change who can listen to your recordings. ${problemFromError(error).message}`,
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
						{guild && !(optOutUnavailable && privacyUnavailable) ? (
							<MenuSection
								aria-label="Recording"
								// Stay open so the check mark visibly follows the change.
								shouldCloseOnSelect={false}
								selectionMode="multiple"
								selectedKeys={[
									...(recorded ? [RECORD_ME] : []),
									...(onlyMe ? [ONLY_ME] : []),
								]}
								onSelectionChange={(keys) => {
									const has = (key: string) => keys === "all" || keys.has(key);
									if (optOut && has(RECORD_ME) !== recorded)
										void setRecorded(has(RECORD_ME));
									if (privacy && has(ONLY_ME) !== onlyMe)
										void setOnlyMe(has(ONLY_ME));
								}}
							>
								{/* Present while their state loads, so the open menu never
								    changes size underneath the pointer. */}
								{!optOutUnavailable && (
									<MenuItem
										id={RECORD_ME}
										isDisabled={!optOut}
										textValue={`Record my voice in ${guild.name}`}
									>
										{({ isSelected }) => (
											<CheckLabel isSelected={isSelected}>
												Record my voice in {guild.name}
											</CheckLabel>
										)}
									</MenuItem>
								)}
								{!privacyUnavailable && (
									<MenuItem
										id={ONLY_ME}
										isDisabled={!privacy}
										textValue={`Only I can listen to my recordings in ${guild.name}`}
									>
										{({ isSelected }) => (
											<CheckLabel isSelected={isSelected}>
												Only I can listen to my recordings in {guild.name}
											</CheckLabel>
										)}
									</MenuItem>
								)}
							</MenuSection>
						) : null}
						<MenuSection aria-label="Account">
							<MenuItem id={DEVICES}>Signed-in devices</MenuItem>
							<MenuItem id={LOG_OUT}>Log out</MenuItem>
							<MenuItem id={LOG_OUT_EVERYWHERE}>Log out everywhere</MenuItem>
						</MenuSection>
					</Menu>
				</Popover>
			</MenuTrigger>
			{devicesOpen && <SessionsDialog onClose={() => setDevicesOpen(false)} />}
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

function CheckLabel(props: { isSelected: boolean; children: ReactNode }) {
	return (
		<span className="flex items-center gap-2">
			<Check
				aria-hidden="true"
				size={16}
				className={cn("shrink-0", !props.isSelected && "invisible")}
			/>
			<span>{props.children}</span>
		</span>
	);
}
