import * as React from "react";
import {
	Avatar,
	Box,
	IconButton,
	Menu,
	MenuItem,
	Tooltip,
	Typography,
} from "../shared/ui";
import { settings } from "./constants";

export function UserMenu() {
	const [anchorElUser, setAnchorElUser] = React.useState<null | HTMLElement>(
		null,
	);

	return (
		<Box className="flex items-center" sx={{ flexGrow: 0 }}>
			<Tooltip title="Open settings">
				<IconButton
					size="medium"
					variant="ghost"
					onClick={(e: React.MouseEvent<HTMLButtonElement>) =>
						setAnchorElUser(e.currentTarget)
					}
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
