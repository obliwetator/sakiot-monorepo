import Avatar from "@mui/material/Avatar";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Menu from "@mui/material/Menu";
import MenuItem from "@mui/material/MenuItem";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import * as React from "react";
import { settings } from "./constants";

export function UserMenu() {
	const [anchorElUser, setAnchorElUser] = React.useState<null | HTMLElement>(
		null,
	);

	return (
		<Box sx={{ flexGrow: 0 }}>
			<Tooltip title="Open settings">
				<IconButton
					onClick={(e) => setAnchorElUser(e.currentTarget)}
					sx={{ p: 0 }}
				>
					<Avatar alt="Remy Sharp" src="/pepega.png" />
				</IconButton>
			</Tooltip>
			<Menu
				sx={{ mt: "45px" }}
				id="menu-appbar"
				anchorEl={anchorElUser}
				anchorOrigin={{ vertical: "top", horizontal: "right" }}
				keepMounted
				transformOrigin={{ vertical: "top", horizontal: "right" }}
				open={Boolean(anchorElUser)}
				onClose={() => setAnchorElUser(null)}
			>
				{settings.map((setting) => (
					<MenuItem key={setting} onClick={() => setAnchorElUser(null)}>
						<Typography textAlign="center">{setting}</Typography>
					</MenuItem>
				))}
			</Menu>
		</Box>
	);
}
