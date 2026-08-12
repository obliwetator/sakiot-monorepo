import RemoveRedEyeIcon from "@mui/icons-material/RemoveRedEye";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Dialog from "@mui/material/Dialog";
import DialogActions from "@mui/material/DialogActions";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import IconButton from "@mui/material/IconButton";
import List from "@mui/material/List";
import ListItem from "@mui/material/ListItem";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemText from "@mui/material/ListItemText";
import Paper from "@mui/material/Paper";
import Stack from "@mui/material/Stack";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import Typography from "@mui/material/Typography";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import type { GuildRole } from "../../app/apiSlice";
import {
	useGetGuildRolesQuery,
	useGetRoleMembersQuery,
	useGetRoleViewQuery,
} from "../../app/apiSlice";
import { PATH_PREFIX_FOR_LOGGED_USERS } from "../../Constants";
import { roleSwatchBackground, roleTextStyle } from "./roleColors";

function RolePreviewDialog(props: {
	open: boolean;
	guildId: string;
	role: GuildRole | null;
	onClose: () => void;
}) {
	const { open, guildId, role, onClose } = props;
	const navigate = useNavigate();
	const {
		data: roleView,
		isLoading,
		isError,
	} = useGetRoleViewQuery(
		{ guild_id: guildId, role_id: role?.role_id ?? "" },
		{ skip: !open || !role },
	);

	const openAudioPreview = () => {
		if (!role) return;
		onClose();
		navigate(
			`${PATH_PREFIX_FOR_LOGGED_USERS}/${guildId}/audio?as_role=${role.role_id}`,
		);
	};

	return (
		<Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
			<DialogTitle>
				{role ? (
					<>
						View as <em>{role.name}</em>
					</>
				) : (
					"View as role"
				)}
			</DialogTitle>
			<DialogContent dividers>
				{isLoading ? (
					<Typography>Loading preview…</Typography>
				) : isError ? (
					<Typography color="error">Failed to load preview.</Typography>
				) : (
					roleView && (
						<Stack spacing={1.5}>
							<Box>
								<Typography variant="subtitle2" gutterBottom>
									What this role can see
								</Typography>
								{roleView.can_manage_guild ? (
									<Chip
										size="small"
										label="Can manage the guild (admin pages included)"
										color="warning"
									/>
								) : (
									<Chip
										size="small"
										label="Cannot manage the guild"
										variant="outlined"
									/>
								)}
							</Box>
							{roleView.channels.length === 0 ? (
								<Typography variant="body2" color="text.secondary">
									No voice channels visible to this role.
								</Typography>
							) : (
								<Box>
									<Typography variant="subtitle2" gutterBottom>
										Visible voice channels ({roleView.channels.length})
									</Typography>
									<List dense disablePadding>
										{roleView.channels.map((channel) => (
											<ListItem
												key={channel.channel_id}
												disablePadding
												secondaryAction={
													<Chip
														size="small"
														variant="outlined"
														color={channel.can_join ? "success" : "default"}
														label={
															channel.can_join ? "Can join" : "Visible only"
														}
													/>
												}
											>
												<ListItemButton dense>
													<ListItemText
														primary={channel.name || channel.channel_id}
														secondary={channel.channel_id}
													/>
												</ListItemButton>
											</ListItem>
										))}
									</List>
								</Box>
							)}
							<Alert severity="info" variant="outlined">
								Open the audio preview to browse recordings, clips and stamps as
								this role. Channels marked "Visible only" won't appear there —
								playback needs join permission, which the role lacks.
							</Alert>
						</Stack>
					)
				)}
			</DialogContent>
			<DialogActions>
				<Button onClick={onClose}>Close</Button>
				<Button
					variant="contained"
					startIcon={<RemoveRedEyeIcon />}
					onClick={openAudioPreview}
					disabled={!role}
				>
					Open audio preview
				</Button>
			</DialogActions>
		</Dialog>
	);
}

export function GuildMembers() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";

	const {
		data: roles,
		isLoading: loadingRoles,
		isError: rolesError,
		error: rolesErrorMessage,
	} = useGetGuildRolesQuery(gid, { skip: !gid });
	const [selectedRole, setSelectedRole] = useState<GuildRole | null>(null);
	const [previewRole, setPreviewRole] = useState<GuildRole | null>(null);

	useEffect(() => {
		const stillListed = (roles ?? []).some(
			(role) => role.role_id === selectedRole?.role_id,
		);
		if (!stillListed) setSelectedRole(roles?.[0] ?? null);
	}, [roles, selectedRole]);

	const {
		data: members,
		isLoading: loadingMembers,
		isError: membersError,
		error: membersErrorMessage,
	} = useGetRoleMembersQuery(
		{ guild_id: gid, role_id: selectedRole?.role_id ?? "" },
		{ skip: !gid || !selectedRole },
	);

	if (!gid) return <Box p={2}>Missing guild id.</Box>;

	return (
		<Box p={2}>
			<Typography variant="h5" gutterBottom>
				Members &amp; roles
			</Typography>

			{loadingRoles ? (
				<Typography>Loading roles…</Typography>
			) : rolesError ? (
				<Typography color="error">
					Failed to load roles: {JSON.stringify(rolesErrorMessage)}
				</Typography>
			) : (
				<Stack
					direction={{ xs: "column", md: "row" }}
					spacing={2}
					alignItems="stretch"
				>
					<Paper variant="outlined" sx={{ minWidth: { md: 280 } }}>
						<List dense disablePadding>
							{(roles ?? []).map((role) => (
								<ListItem
									key={role.role_id}
									disablePadding
									secondaryAction={
										<IconButton
											size="small"
											aria-label={`View as ${role.name}`}
											title="View server as this role"
											onClick={() => setPreviewRole(role)}
										>
											<RemoveRedEyeIcon fontSize="small" />
										</IconButton>
									}
								>
									<ListItemButton
										selected={selectedRole?.role_id === role.role_id}
										onClick={() => setSelectedRole(role)}
									>
										<Box
											component="span"
											aria-hidden="true"
											sx={{
												width: 14,
												height: 14,
												borderRadius: "4px",
												flexShrink: 0,
												mr: 1.5,
												background: roleSwatchBackground(role),
											}}
										/>
										<ListItemText
											primary={role.name}
											secondary={
												<Chip
													size="small"
													label={`${role.member_count} member${role.member_count === 1 ? "" : "s"}`}
												/>
											}
										/>
									</ListItemButton>
								</ListItem>
							))}
							{(!roles || roles.length === 0) && (
								<ListItemText sx={{ p: 2 }}>
									<Typography variant="body2" color="text.secondary">
										No roles in this guild yet.
									</Typography>
								</ListItemText>
							)}
						</List>
					</Paper>

					<TableContainer component={Paper} variant="outlined" sx={{ flex: 1 }}>
						{selectedRole && (
							<Typography variant="subtitle1" sx={{ p: 2 }}>
								<span style={roleTextStyle(selectedRole)}>
									{selectedRole.name}
								</span>{" "}
								— {members?.length ?? 0}{" "}
								{members?.length === 1 ? "member" : "members"}
							</Typography>
						)}
						{loadingMembers ? (
							<Typography sx={{ p: 2 }}>Loading members…</Typography>
						) : membersError ? (
							<Typography color="error" sx={{ p: 2 }}>
								Failed to load members: {JSON.stringify(membersErrorMessage)}
							</Typography>
						) : (
							<Table size="small">
								<TableHead>
									<TableRow>
										<TableCell>Name</TableCell>
										<TableCell>User ID</TableCell>
									</TableRow>
								</TableHead>
								<TableBody>
									{(members ?? []).map((member) => (
										<TableRow key={member.user_id}>
											<TableCell>
												{member.name ?? <em>unknown user</em>}
											</TableCell>
											<TableCell>{member.user_id}</TableCell>
										</TableRow>
									))}
									{(!members || members.length === 0) && (
										<TableRow>
											<TableCell colSpan={2}>
												<Typography variant="body2" color="text.secondary">
													{selectedRole
														? "No members hold this role."
														: "Select a role to see its members."}
												</Typography>
											</TableCell>
										</TableRow>
									)}
								</TableBody>
							</Table>
						)}
					</TableContainer>
				</Stack>
			)}

			<RolePreviewDialog
				open={previewRole !== null}
				guildId={gid}
				role={previewRole}
				onClose={() => setPreviewRole(null)}
			/>
		</Box>
	);
}
