import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import {
	useDeleteGuildVoiceSettingsMutation,
	useGetGuildVoiceSettingsQuery,
	useSetGuildVoiceSettingsMutation,
} from "../../app/apiSlice";
import {
	Alert,
	Box,
	Button,
	Paper,
	Stack,
	LegacyTextField as TextField,
	Typography,
} from "../../shared/ui";

const MIN_PENDING_SECONDS = 60;

export function GuildVoiceSettingsPage() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const guildId = guild_id ?? "";
	const { data, isLoading, isError } = useGetGuildVoiceSettingsQuery(guildId, {
		skip: !guildId,
	});
	const [save, saveState] = useSetGuildVoiceSettingsMutation();
	const [reset, resetState] = useDeleteGuildVoiceSettingsMutation();
	const [seconds, setSeconds] = useState("21600");
	const [validation, setValidation] = useState<string | null>(null);

	useEffect(() => {
		if (data) setSeconds(String(data.pending_cap_seconds));
	}, [data]);

	const handleSave = async () => {
		const parsed = Number(seconds);
		if (
			!Number.isInteger(parsed) ||
			!Number.isFinite(parsed) ||
			parsed < MIN_PENDING_SECONDS
		) {
			setValidation(
				`Pending timeout must be at least ${MIN_PENDING_SECONDS} seconds.`,
			);
			return;
		}
		setValidation(null);
		await save({ guild_id: guildId, pending_cap_seconds: parsed });
	};

	const handleReset = async () => {
		setValidation(null);
		const restored = await reset(guildId).unwrap();
		setSeconds(String(restored.pending_cap_seconds));
	};

	if (!guildId) return <Box p={2}>Missing guild id.</Box>;

	return (
		<Box p={2} sx={{ maxWidth: 760 }}>
			<Typography variant="h5" gutterBottom>
				Voice Settings
			</Typography>
			<Paper sx={{ p: 3 }}>
				<Typography variant="h6" gutterBottom>
					Pending recording timeout
				</Typography>
				<Typography color="text.secondary" sx={{ mb: 2 }}>
					Guilds without an AFK channel finalize users who do not follow the bot
					after this cap. Disconnect and AFK events still use a 60-second grace.
					Guilds with an AFK channel have no absolute cap.
				</Typography>

				{isLoading && <Typography>Loading voice settings…</Typography>}
				{isError && (
					<Alert severity="error">Could not load voice settings.</Alert>
				)}
				{data && (
					<Stack spacing={2}>
						<TextField
							label="Pending cap (seconds)"
							type="number"
							value={seconds}
							onChange={(event) => setSeconds(event.target.value)}
							inputProps={{ min: MIN_PENDING_SECONDS, step: 60 }}
							helperText={
								data.is_default
									? "Using six-hour default."
									: "Guild override active."
							}
						/>
						<Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
							<Button
								variant="contained"
								onClick={handleSave}
								disabled={saveState.isLoading}
							>
								Save override
							</Button>
							<Button
								variant="outlined"
								onClick={handleReset}
								disabled={resetState.isLoading || data.is_default}
							>
								Restore six-hour default
							</Button>
						</Stack>
						{validation && <Alert severity="warning">{validation}</Alert>}
						{saveState.isSuccess && <Alert severity="success">Saved.</Alert>}
						{saveState.isError && <Alert severity="error">Save failed.</Alert>}
						{resetState.isSuccess && (
							<Alert severity="success">Default restored.</Alert>
						)}
					</Stack>
				)}
			</Paper>
		</Box>
	);
}
