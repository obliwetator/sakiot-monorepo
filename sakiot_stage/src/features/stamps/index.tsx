import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Paper from "@mui/material/Paper";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import Typography from "@mui/material/Typography";
import { useSelector } from "react-redux";
import { useNavigate } from "react-router-dom";
import { useGetStampsQuery } from "../../app/apiSlice";
import type { RootState } from "../../store";
import { formatDuration } from "../../utils/formatTime";

function formatTimestamp(ms: number): string {
	return new Date(ms).toLocaleString();
}

export function Stamps() {
	const navigate = useNavigate();
	const guild = useSelector((s: RootState) => s.app.guildSelected);
	const guildId = guild?.id ?? "";

	const { data, isLoading, isError, error } = useGetStampsQuery(guildId, {
		skip: !guildId,
	});

	if (!guildId) {
		return (
			<Box p={3}>
				<Typography color="text.secondary">
					Select a guild from the top navbar to view stamps.
				</Typography>
			</Box>
		);
	}

	if (isLoading) {
		return (
			<Box p={3}>
				<Typography>Loading stamps…</Typography>
			</Box>
		);
	}

	if (isError) {
		return (
			<Box p={3}>
				<Typography color="error">
					Failed to load stamps: {JSON.stringify(error)}
				</Typography>
			</Box>
		);
	}

	const rows = data ?? [];

	return (
		<Box sx={{ p: { xs: 1.5, md: 3 }, maxWidth: 1400 }}>
			<Typography variant="h4" fontWeight={700} gutterBottom>
				Stamps {guild ? `— ${guild.name}` : ""}
			</Typography>
			<Typography color="text.secondary" sx={{ mb: 2 }}>
				{rows.length} stamp{rows.length === 1 ? "" : "s"} (newest first, max
				500).
			</Typography>

			{rows.length === 0 ? (
				<Typography color="text.secondary">No stamps yet.</Typography>
			) : (
				<TableContainer component={Paper} variant="outlined">
					<Table size="small">
						<TableHead>
							<TableRow>
								<TableCell>ID</TableCell>
								<TableCell>Absolute Time</TableCell>
								<TableCell>Relative Time</TableCell>
								<TableCell>Target</TableCell>
								<TableCell>Stamper</TableCell>
								<TableCell>Channel</TableCell>
								<TableCell align="right">Offset (ms)</TableCell>
								<TableCell align="right">Audio File ID</TableCell>
								<TableCell>Note</TableCell>
								<TableCell>Created</TableCell>
							</TableRow>
						</TableHead>
						<TableBody>
							{rows.map((s) => (
								<TableRow key={s.id} hover>
									<TableCell>{s.id}</TableCell>
									<TableCell>{formatTimestamp(s.stamp_ts)}</TableCell>
									<TableCell>
										{s.start_ts != null ? (
											<Button
												size="small"
												onClick={() => {
													if (s.file_name && s.year && s.month) {
														const relSecs =
															(s.stamp_ts - (s.start_ts ?? 0) + s.offset_ms) /
															1000;
														navigate(
															`/dashboard/${guildId}/audio/${s.channel_id}/${s.year}/${s.month}/${s.file_name}?t=${relSecs}`,
														);
													}
												}}
												disabled={!s.file_name}
											>
												{formatDuration(
													(s.stamp_ts - s.start_ts + s.offset_ms) / 1000,
												)}
											</Button>
										) : (
											<span style={{ opacity: 0.5 }}>—</span>
										)}
									</TableCell>
									<TableCell>
										<div>{s.target_name ?? s.target_user_id}</div>
										{s.target_name && (
											<div style={{ fontSize: 11, opacity: 0.6 }}>
												{s.target_user_id}
											</div>
										)}
									</TableCell>
									<TableCell>
										<div>{s.stamper_name ?? s.stamper_user_id}</div>
										{s.stamper_name && (
											<div style={{ fontSize: 11, opacity: 0.6 }}>
												{s.stamper_user_id}
											</div>
										)}
									</TableCell>
									<TableCell>
										<div>{s.channel_name ?? s.channel_id}</div>
										{s.channel_name && (
											<div style={{ fontSize: 11, opacity: 0.6 }}>
												{s.channel_id}
											</div>
										)}
									</TableCell>
									<TableCell align="right">{s.offset_ms}</TableCell>
									<TableCell align="right">
										{s.audio_file_id ?? <span style={{ opacity: 0.5 }}>—</span>}
									</TableCell>
									<TableCell>{s.note ?? ""}</TableCell>
									<TableCell>
										{new Date(s.created_at).toLocaleString()}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</TableContainer>
			)}
		</Box>
	);
}
