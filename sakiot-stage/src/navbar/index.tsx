import * as React from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../Constants";
import Login from "../login/login";
import { GuildSelect } from "../shared/GuildSelect";
import { isGuildAdmin } from "../shared/permissions";
import { Button, cn, Drawer, IconButton } from "../shared/ui";
import { type PageName, pages } from "./constants";
import { MobileDrawer } from "./MobileDrawer";
import { UserMenu } from "./UserMenu";

function ResponsiveAppBar(props: {
	isLoggedIn: boolean;
	setIsLoggedIn: React.Dispatch<React.SetStateAction<boolean>>;
	guildSelected: UserGuilds | null;
	setGuildSelected: (guild: UserGuilds | null) => void;
	userGuilds: UserGuilds[] | null;
}) {
	const navigate = useNavigate();
	const location = useLocation();
	const [drawerOpen, setDrawerOpen] = React.useState(false);
	// The audio dashboard keeps the same navigation controls as desktop on
	// narrow screens. Its file tree is part of the page flow, so a second
	// navigation drawer would make the mobile layout needlessly indirect.
	const useInlineNavigation = location.pathname.includes("/audio");

	const navigateTo = (name: PageName) => {
		if (!props.guildSelected && name !== "Stamps") {
			navigate(`${PATH_PREFIX_FOR_LOGGED_USERS}`);
			return;
		}
		switch (name) {
			case "Admin":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/admin/cooldowns`,
				);
				break;
			case "Voice Settings":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/admin/voice-settings`,
				);
				break;
			case "Members":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/members`,
				);
				break;
			case "Audio":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/audio`,
				);
				break;
			case "Clips":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/clips`,
				);
				break;
			case "Clip Editor":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/clips/editor`,
				);
				break;
			case "Stamps":
				navigate(
					props.guildSelected?.id
						? `/stamps/${props.guildSelected.id}`
						: `/stamps`,
				);
				break;
		}
	};

	const handleDrawerNavClick = (name: PageName) => {
		setDrawerOpen(false);
		navigateTo(name);
	};

	const visiblePages: PageName[] = isGuildAdmin(props.guildSelected)
		? [...pages, "Admin", "Voice Settings", "Members"]
		: pages;

	return (
		<header className="w-full border-b border-ui-border bg-header text-fg shadow-sm static">
			<div
				className={cn("mx-auto w-full px-4 sm:px-6 [max-width:xl]", "min-w-0")}
			>
				<div
					className={cn(
						"flex min-h-14 items-center justify-between gap-4 px-4 sm:min-h-16",
						useInlineNavigation ? "flex-wrap gap-y-1 py-1" : undefined,
					)}
				>
					<div
						className={cn(
							"mr-2",
							useInlineNavigation ? "hidden" : "flex min-[900px]:hidden",
						)}
					>
						<IconButton
							aria-label="open navigation"
							className="text-white"
							size="lg"
							onPress={() => setDrawerOpen(true)}
						>
							<span aria-hidden="true" className="relative block size-6">
								<span className="absolute left-[2px] top-[6px] h-0.5 w-[18px] bg-current" />
								<span className="absolute left-[2px] top-[11px] h-0.5 w-[18px] bg-current" />
								<span className="absolute left-[2px] top-[16px] h-0.5 w-[18px] bg-current" />
							</span>
						</IconButton>
					</div>

					<h6
						className={cn(
							"font-medium tracking-[0.001em] text-xl truncate grow [color:inherit]",
							useInlineNavigation ? "hidden" : "flex min-[900px]:hidden",
						)}
					>
						{props.guildSelected?.name ?? "Sakiot"}
					</h6>

					<div
						className={cn(
							"grow min-w-0 items-center overflow-x-hidden",
							useInlineNavigation ? "flex" : "hidden min-[900px]:flex",
							useInlineNavigation ? "flex-wrap" : "flex-nowrap",
						)}
					>
						{visiblePages.map((page) => (
							<Button
								key={page}
								className="my-2 shrink-0 min-w-16 whitespace-nowrap rounded-sm border-0 px-2 text-sm font-medium uppercase tracking-normal text-white"
								variant="ghost"
								size="sm"
								onPress={() => navigateTo(page)}
							>
								{page}
							</Button>
						))}
						{props.guildSelected ? (
							<div className="flex items-center gap-2 px-4 py-1.5">
								{useInlineNavigation ? (
									<span className="max-[899px]:hidden">Select Server:</span>
								) : (
									"Select Server:"
								)}
								<GuildSelect
									guildSelected={props.guildSelected}
									setGuildSelected={props.setGuildSelected}
									userGuilds={props.userGuilds}
								/>
							</div>
						) : null}
					</div>

					<div
						className={cn(
							"shrink-0",
							useInlineNavigation ? "flex" : "hidden min-[900px]:flex",
						)}
					>
						<Login
							isLoggedIn={props.isLoggedIn}
							setIsLoggedIn={props.setIsLoggedIn}
						/>
					</div>

					<UserMenu />
				</div>
			</div>

			{!useInlineNavigation && (
				<Drawer
					className="block min-[900px]:hidden"
					isOpen={drawerOpen}
					side={"left"}
					onOpenChange={(isOpen) => {
						if (!isOpen) setDrawerOpen(false);
					}}
				>
					<MobileDrawer
						isLoggedIn={props.isLoggedIn}
						setIsLoggedIn={props.setIsLoggedIn}
						guildSelected={props.guildSelected}
						setGuildSelected={props.setGuildSelected}
						userGuilds={props.userGuilds}
						visiblePages={visiblePages}
						onNavigate={handleDrawerNavClick}
					/>
				</Drawer>
			)}
		</header>
	);
}

export default ResponsiveAppBar;
