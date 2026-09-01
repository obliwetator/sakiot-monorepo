import * as React from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../Constants";
import Login from "../login/login";
import { BasicSelect } from "../shared/BasicSelect";
import { isGuildAdmin } from "../shared/permissions";
import {
	AppBar,
	Box,
	Button,
	Container,
	Drawer,
	IconButton,
	MenuItem,
	Toolbar,
	Typography,
} from "../shared/ui";
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
		<AppBar position="static">
			<Container maxWidth="xl" className="min-w-0">
				<Toolbar
					disableGutters
					className={useInlineNavigation ? "flex-wrap gap-y-1 py-1" : undefined}
				>
					<Box
						sx={{
							display: useInlineNavigation
								? "none"
								: { xs: "flex", md: "none" },
							mr: 1,
						}}
					>
						<IconButton
							size="large"
							aria-label="open navigation"
							onClick={() => setDrawerOpen(true)}
							color="inherit"
							className="text-white"
						>
							<span aria-hidden="true" className="relative block size-6">
								<span className="absolute left-[2px] top-[6px] h-0.5 w-[18px] bg-current" />
								<span className="absolute left-[2px] top-[11px] h-0.5 w-[18px] bg-current" />
								<span className="absolute left-[2px] top-[16px] h-0.5 w-[18px] bg-current" />
							</span>
						</IconButton>
					</Box>

					<Typography
						variant="h6"
						noWrap
						sx={{
							flexGrow: 1,
							display: useInlineNavigation
								? "none"
								: { xs: "flex", md: "none" },
							color: "inherit",
						}}
					>
						{props.guildSelected?.name ?? "Sakiot"}
					</Typography>

					<Box
						sx={{
							flexGrow: 1,
							minWidth: 0,
							display: useInlineNavigation
								? "flex"
								: { xs: "none", md: "flex" },
							alignItems: "center",
							flexWrap: useInlineNavigation ? "wrap" : "nowrap",
							overflowX: "hidden",
						}}
					>
						{visiblePages.map((page) => (
							<Button
								key={page}
								variant="text"
								size="small"
								onClick={() => navigateTo(page)}
								className="my-2 shrink-0 min-w-16 whitespace-nowrap rounded-sm border-0 px-2 text-sm font-medium uppercase tracking-normal text-white"
							>
								{page}
							</Button>
						))}
						{props.guildSelected ? (
							<MenuItem
								className={useInlineNavigation ? "min-w-0 shrink" : undefined}
							>
								{useInlineNavigation ? (
									<span className="max-[899px]:hidden">Select Server:</span>
								) : (
									"Select Server:"
								)}
								<BasicSelect
									guildSelected={props.guildSelected}
									setGuildSelected={props.setGuildSelected}
									userGuilds={props.userGuilds}
								/>
							</MenuItem>
						) : null}
					</Box>

					<Box
						sx={{
							display: useInlineNavigation
								? "flex"
								: { xs: "none", md: "flex" },
							flexShrink: 0,
						}}
					>
						<Login
							isLoggedIn={props.isLoggedIn}
							setIsLoggedIn={props.setIsLoggedIn}
						/>
					</Box>

					<UserMenu />
				</Toolbar>
			</Container>

			{!useInlineNavigation && (
				<Drawer
					anchor="left"
					open={drawerOpen}
					onClose={() => setDrawerOpen(false)}
					sx={{ display: { xs: "block", md: "none" } }}
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
		</AppBar>
	);
}

export default ResponsiveAppBar;
