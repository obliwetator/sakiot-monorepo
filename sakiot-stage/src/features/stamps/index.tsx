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
import { useNavigate, useParams } from "react-router-dom";
import { useGetStampsQuery } from "../../app/apiSlice";
import { useAsRole } from "../../app/useAsRole";
import type { RootState } from "../../store";
import { formatDuration } from "../../utils/formatTime";
import { ViewAsRoleBanner } from "../members/ViewAsRoleBanner";
import { buildStampPlaybackTarget } from "./stampNavigation";

function formatTimestamp(ms: number): string {
	return new Date(ms).toLocaleString();
}

export function Stamps() {
	const navigate = useNavigate();
	const params = useParams();
	const guild = useSelector((s: RootState) => s.app.guildSelected);
	const guildId = params.guild_id ?? "";
	const guildName = guild?.id === guildId ? guild.name : undefined;
	const { asRoleArg } = useAsRole();

	const { data, isLoading, isError, error } = useGetStampsQuery(
		{ guild_id: guildId, ...asRoleArg },
		{
			skip: !guildId,
		},
	);

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
			<ViewAsRoleBanner guildId={guildId} />
			<Typography variant="h4" fontWeight={700} gutterBottom>
				Stamps {guildName ? `— ${guildName}` : ""}
			</Typography>
			<Typography color="text.secondary" sx={{ mb: 2 }}>
				{rows.length} stamp{rows.length === 1 ? "" : "s"} (newest first, max
				500). Session links open the complete logical recording. Audio file IDs
				identify the physical fragment containing each stamp.
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
								<TableCell>Session ID</TableCell>
								<TableCell>Audio File ID</TableCell>
								<TableCell>Note</TableCell>
								<TableCell>Created</TableCell>
							</TableRow>
						</TableHead>
						<TableBody>
							{rows.map((s) => {
								const playbackTarget = buildStampPlaybackTarget(s, guildId);
								const fragmentNumber =
									s.segment_index == null ? null : s.segment_index + 1;
								const sessionPath = s.recording_session_id
									? `/dashboard/${encodeURIComponent(guildId)}/audio/session/${encodeURIComponent(s.recording_session_id)}`
									: null;
								return (
									<TableRow key={s.id} hover>
										<TableCell>{s.id}</TableCell>
										<TableCell>{formatTimestamp(s.stamp_ts)}</TableCell>
										<TableCell>
											{playbackTarget ? (
												<Button
													size="small"
													onClick={() => navigate(playbackTarget.path)}
													title={
														playbackTarget.scope === "session"
															? "Open complete logical session"
															: "Open legacy physical fragment"
													}
												>
													{formatDuration(playbackTarget.relativeSeconds)}
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
										<TableCell>
											{sessionPath ? (
												<Button
													size="small"
													onClick={() =>
														navigate(
															playbackTarget?.scope === "session"
																? playbackTarget.path
																: sessionPath,
														)
													}
												>
													{s.recording_session_id}
												</Button>
											) : (
												<span style={{ opacity: 0.5 }}>—</span>
											)}
										</TableCell>
										<TableCell>
											{s.audio_file_id ? (
												<Box>
													<Typography variant="body2">
														{s.audio_file_id}
													</Typography>
													<Typography
														variant="caption"
														color="text.secondary"
														display="block"
													>
														{s.recording_session_id
															? `Fragment ${fragmentNumber ?? "?"}${
																	s.session_fragment_count
																		? ` of ${s.session_fragment_count}`
																		: ""
																}`
															: "Legacy physical file"}
													</Typography>
												</Box>
											) : (
												<span style={{ opacity: 0.5 }}>—</span>
											)}
										</TableCell>
										<TableCell>{s.note ?? ""}</TableCell>
										<TableCell>
											{new Date(s.created_at).toLocaleString()}
										</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				</TableContainer>
			)}
		</Box>
	);
}
