import DeleteIcon from "@mui/icons-material/Delete";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import IconButton from "@mui/material/IconButton";
import Paper from "@mui/material/Paper";
import Stack from "@mui/material/Stack";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import {
	useDeleteUserOverrideMutation,
	useGetGuildCooldownQuery,
	useListUserOverridesQuery,
	useSetGuildCooldownMutation,
	useSetUserOverrideMutation,
} from "../../app/apiSlice";

export function GuildAdminCooldowns() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";

	const { data: guildCooldown, isLoading: loadingGuild } =
		useGetGuildCooldownQuery(gid, { skip: !gid });
	const { data: overrides, isLoading: loadingOverrides } =
		useListUserOverridesQuery(gid, { skip: !gid });
	const [setGuildCooldown, setGuildState] = useSetGuildCooldownMutation();
	const [setUserOverride, setOverrideState] = useSetUserOverrideMutation();
	const [deleteUserOverride] = useDeleteUserOverrideMutation();

	const [guildSeconds, setGuildSeconds] = useState<string>("0");
	const [newUserId, setNewUserId] = useState<string>("");
	const [newSeconds, setNewSeconds] = useState<string>("0");
	const [formError, setFormError] = useState<string | null>(null);

	useEffect(() => {
		if (guildCooldown) setGuildSeconds(String(guildCooldown.cooldown_seconds));
	}, [guildCooldown]);

	const parseSeconds = (s: string): number | null => {
		const n = Number(s);
		return Number.isFinite(n) && Number.isInteger(n) && n >= 0 ? n : null;
	};

	const handleSaveGuild = async () => {
		const n = parseSeconds(guildSeconds);
		if (n === null) {
			setFormError("Cooldown must be a non-negative integer.");
			return;
		}
		setFormError(null);
		await setGuildCooldown({ guild_id: gid, cooldown_seconds: n });
	};

	const handleAddOverride = async () => {
		const n = parseSeconds(newSeconds);
		if (!newUserId.trim() || n === null) {
			setFormError("Provide a user id and non-negative integer seconds.");
			return;
		}
		setFormError(null);
		await setUserOverride({
			guild_id: gid,
			user_id: newUserId.trim(),
			cooldown_seconds: n,
		});
		setNewUserId("");
		setNewSeconds("0");
	};

	const handleDelete = async (user_id: number) => {
		await deleteUserOverride({ guild_id: gid, user_id: String(user_id) });
	};

	if (!gid) return <Box p={2}>Missing guild id.</Box>;

	return (
		<Box p={2}>
			<Typography variant="h5" gutterBottom>
				Jam cooldowns
			</Typography>

			<Paper sx={{ p: 2, mb: 3 }}>
				<Typography variant="h6" gutterBottom>
					Guild default
				</Typography>
				{loadingGuild ? (
					<Typography>Loading…</Typography>
				) : (
					<Stack
						direction={{ xs: "column", sm: "row" }}
						spacing={2}
						alignItems={{ xs: "stretch", sm: "center" }}
					>
						<TextField
							label="Cooldown (seconds)"
							type="number"
							value={guildSeconds}
							onChange={(e) => setGuildSeconds(e.target.value)}
							inputProps={{ min: 0 }}
							size="small"
						/>
						<Button
							variant="contained"
							onClick={handleSaveGuild}
							disabled={setGuildState.isLoading}
						>
							Save
						</Button>
						{setGuildState.isSuccess && <Alert severity="success">Saved</Alert>}
						{setGuildState.isError && (
							<Alert severity="error">Save failed</Alert>
						)}
					</Stack>
				)}
				<Typography
					variant="caption"
					color="text.secondary"
					sx={{ mt: 1, display: "block" }}
				>
					0 disables the cooldown for this guild.
				</Typography>
			</Paper>

			<Paper sx={{ p: 2 }}>
				<Typography variant="h6" gutterBottom>
					Per-user overrides
				</Typography>

				<Stack
					direction={{ xs: "column", sm: "row" }}
					spacing={2}
					alignItems={{ xs: "stretch", sm: "center" }}
					sx={{ mb: 2 }}
				>
					<TextField
						label="User ID"
						value={newUserId}
						onChange={(e) => setNewUserId(e.target.value)}
						size="small"
					/>
					<TextField
						label="Cooldown (seconds)"
						type="number"
						value={newSeconds}
						onChange={(e) => setNewSeconds(e.target.value)}
						inputProps={{ min: 0 }}
						size="small"
					/>
					<Button
						variant="contained"
						onClick={handleAddOverride}
						disabled={setOverrideState.isLoading}
					>
						Add / Update
					</Button>
				</Stack>

				{formError && (
					<Alert severity="warning" sx={{ mb: 2 }}>
						{formError}
					</Alert>
				)}

				{loadingOverrides ? (
					<Typography>Loading Admin Cooldowns</Typography>
				) : (
					<TableContainer>
						<Table size="small">
							<TableHead>
								<TableRow>
									<TableCell>User ID</TableCell>
									<TableCell align="right">Cooldown (s)</TableCell>
									<TableCell>Updated</TableCell>
									<TableCell align="right">Actions</TableCell>
								</TableRow>
							</TableHead>
							<TableBody>
								{(overrides ?? []).map((o) => (
									<TableRow key={o.user_id}>
										<TableCell>{o.user_id}</TableCell>
										<TableCell align="right">{o.cooldown_seconds}</TableCell>
										<TableCell>
											{new Date(o.updated_at).toLocaleString()}
										</TableCell>
										<TableCell align="right">
											<IconButton
												size="small"
												onClick={() => handleDelete(o.user_id)}
											>
												<DeleteIcon fontSize="small" />
											</IconButton>
										</TableCell>
									</TableRow>
								))}
								{(!overrides || overrides.length === 0) && (
									<TableRow>
										<TableCell colSpan={4}>
											<Typography variant="body2" color="text.secondary">
												No per-user overrides.
											</Typography>
										</TableCell>
									</TableRow>
								)}
							</TableBody>
						</Table>
					</TableContainer>
				)}
			</Paper>
		</Box>
	);
}
