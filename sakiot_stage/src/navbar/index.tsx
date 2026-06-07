import MenuIcon from "@mui/icons-material/Menu";
import AppBar from "@mui/material/AppBar";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Container from "@mui/material/Container";
import Drawer from "@mui/material/Drawer";
import IconButton from "@mui/material/IconButton";
import MenuItem from "@mui/material/MenuItem";
import Toolbar from "@mui/material/Toolbar";
import Typography from "@mui/material/Typography";
import * as React from "react";
import { useNavigate } from "react-router-dom";
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../Constants";
import Login from "../login/login";
import { BasicSelect } from "../shared/BasicSelect";
import { isGuildAdmin } from "../shared/permissions";
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
	const [drawerOpen, setDrawerOpen] = React.useState(false);

	const navigateTo = (name: PageName) => {
		if (!props.guildSelected && name !== "Metrics" && name !== "Stamps") {
			navigate(`${PATH_PREFIX_FOR_LOGGED_USERS}`);
			return;
		}
		switch (name) {
			case "Admin":
				navigate(
					`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.guildSelected?.id}/admin/cooldowns`,
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
			case "Metrics":
				navigate(
					props.guildSelected?.id
						? `/metrics/${props.guildSelected.id}`
						: `/metrics`,
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
		? [...pages, "Admin"]
		: pages;

	return (
		<AppBar position="static">
			<Container maxWidth="xl">
				<Toolbar disableGutters>
					<Box sx={{ display: { xs: "flex", md: "none" }, mr: 1 }}>
						<IconButton
							size="large"
							aria-label="open navigation"
							onClick={() => setDrawerOpen(true)}
							color="inherit"
						>
							<MenuIcon />
						</IconButton>
					</Box>

					<Typography
						variant="h6"
						noWrap
						sx={{
							flexGrow: 1,
							display: { xs: "flex", md: "none" },
							color: "inherit",
						}}
					>
						{props.guildSelected?.name ?? "Sakiot"}
					</Typography>

					<Box
						sx={{
							flexGrow: 1,
							display: { xs: "none", md: "flex" },
							alignItems: "center",
						}}
					>
						{visiblePages.map((page) => (
							<Button
								key={page}
								onClick={() => navigateTo(page)}
								sx={{ my: 2, color: "white", display: "block" }}
							>
								{page}
							</Button>
						))}
						{props.guildSelected ? (
							<MenuItem>
								Select Server:
								<BasicSelect
									guildSelected={props.guildSelected}
									setGuildSelected={props.setGuildSelected}
									userGuilds={props.userGuilds}
								/>
							</MenuItem>
						) : null}
					</Box>

					<Box sx={{ display: { xs: "none", md: "flex" } }}>
						<Login
							isLoggedIn={props.isLoggedIn}
							setIsLoggedIn={props.setIsLoggedIn}
						/>
					</Box>

					<UserMenu />
				</Toolbar>
			</Container>

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
		</AppBar>
	);
}

export default ResponsiveAppBar;
