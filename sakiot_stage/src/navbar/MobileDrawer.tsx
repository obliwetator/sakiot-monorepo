import Avatar from "@mui/material/Avatar";
import Box from "@mui/material/Box";
import Divider from "@mui/material/Divider";
import List from "@mui/material/List";
import ListItem from "@mui/material/ListItem";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemIcon from "@mui/material/ListItemIcon";
import ListItemText from "@mui/material/ListItemText";
import ListSubheader from "@mui/material/ListSubheader";
import Typography from "@mui/material/Typography";
import type * as React from "react";
import type { UserGuilds } from "../Constants";
import Login from "../login/login";
import { BasicSelect } from "../shared/BasicSelect";
import { type PageName, pageIcons } from "./constants";

export function MobileDrawer(props: {
	isLoggedIn: boolean;
	setIsLoggedIn: React.Dispatch<React.SetStateAction<boolean>>;
	guildSelected: UserGuilds | null;
	setGuildSelected: (guild: UserGuilds | null) => void;
	userGuilds: UserGuilds[] | null;
	visiblePages: PageName[];
	onNavigate: (name: PageName) => void;
}) {
	return (
		<Box sx={{ width: 280 }} role="presentation">
			<Box sx={{ p: 2, display: "flex", alignItems: "center", gap: 2 }}>
				<Avatar alt="user" src="/pepega.png" />
				<Typography variant="subtitle1" noWrap>
					{props.isLoggedIn ? "Account" : "Guest"}
				</Typography>
			</Box>
			<Divider />
			{props.userGuilds && props.userGuilds.length > 0 ? (
				<Box sx={{ p: 2 }}>
					<BasicSelect
						guildSelected={props.guildSelected}
						setGuildSelected={props.setGuildSelected}
						userGuilds={props.userGuilds}
					/>
				</Box>
			) : null}
			<Divider />
			<List subheader={<ListSubheader>Navigate</ListSubheader>}>
				{props.visiblePages.map((page) => (
					<ListItem key={page} disablePadding>
						<ListItemButton onClick={() => props.onNavigate(page)}>
							<ListItemIcon>{pageIcons[page]}</ListItemIcon>
							<ListItemText primary={page} />
						</ListItemButton>
					</ListItem>
				))}
			</List>
			<Divider />
			<List subheader={<ListSubheader>Account</ListSubheader>}>
				<ListItem disablePadding>
					<Box sx={{ px: 2, py: 1 }}>
						<Login
							isLoggedIn={props.isLoggedIn}
							setIsLoggedIn={props.setIsLoggedIn}
						/>
					</Box>
				</ListItem>
			</List>
		</Box>
	);
}
