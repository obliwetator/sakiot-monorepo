import Box from "@mui/material/Box";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import Typography from "@mui/material/Typography";
import { GroupPanel, Tile } from "../../shared/primitives";
import { formatTimeSince, formatUptime } from "../../utils/formatTime";
import type { GuildInfo, RecordingMetrics, VoiceState } from "./types";

export function VoiceUsersSection(props: {
	selectedGuild: GuildInfo | null;
	guildRecordingMetrics: RecordingMetrics | null;
	voiceUsers: VoiceState[];
	userStartTimes: Record<string, number>;
	currentTime: number;
}) {
	if (!props.selectedGuild) {
		return (
			<Typography color="text.secondary">
				Select a guild above to view voice users and recording metrics.
			</Typography>
		);
	}

	return (
		<>
			{props.guildRecordingMetrics && (
				<Box sx={{ mb: 2 }}>
					<GroupPanel title="Guild Recording Metrics">
						<Tile
							label="Active Recordings"
							value={props.guildRecordingMetrics.active_recordings}
						/>
						<Tile
							label="Packets Received"
							value={props.guildRecordingMetrics.audio_packets_received.toLocaleString()}
						/>
						<Tile
							label="Packets Dropped"
							value={props.guildRecordingMetrics.audio_packets_dropped.toLocaleString()}
							warn={props.guildRecordingMetrics.audio_packets_dropped > 0}
						/>
						<Tile
							label="FFmpeg Spawn Failures"
							value={props.guildRecordingMetrics.ffmpeg_spawn_failures}
							warn={props.guildRecordingMetrics.ffmpeg_spawn_failures > 0}
						/>
						<Tile
							label="FFmpeg Crashes"
							value={props.guildRecordingMetrics.ffmpeg_process_crashes}
							warn={props.guildRecordingMetrics.ffmpeg_process_crashes > 0}
						/>
						<Tile
							label="Last Voice Packet"
							value={formatTimeSince(
								props.guildRecordingMetrics.last_voice_packet_time,
								props.currentTime,
							)}
						/>
					</GroupPanel>
				</Box>
			)}

			{props.voiceUsers.length === 0 ? (
				<Typography color="text.secondary">
					No users currently in voice channels.
				</Typography>
			) : (
				<Box sx={{ display: "flex", flexWrap: "wrap", gap: 2 }}>
					{props.voiceUsers.map((user) => (
						<Box sx={{ flex: "1 1 280px", maxWidth: 360 }} key={user.user_id}>
							<Card variant="outlined">
								<CardContent>
									<Typography variant="subtitle2" fontWeight={700} gutterBottom>
										{user.user_id}
									</Typography>
									<Typography variant="body2" color="text.secondary">
										Channel: {user.channel_id}
									</Typography>
									{props.userStartTimes[user.user_id] && (
										<Typography variant="body2" color="text.secondary">
											Active for:{" "}
											{formatUptime(
												props.currentTime - props.userStartTimes[user.user_id],
											)}
										</Typography>
									)}
									<Box
										sx={{ display: "flex", gap: 1, mt: 1, flexWrap: "wrap" }}
									>
										{user.mute && (
											<Typography
												variant="caption"
												sx={{
													bgcolor: "warning.dark",
													px: 0.75,
													py: 0.25,
													borderRadius: 0.5,
												}}
											>
												Server Muted
											</Typography>
										)}
										{user.deaf && (
											<Typography
												variant="caption"
												sx={{
													bgcolor: "warning.dark",
													px: 0.75,
													py: 0.25,
													borderRadius: 0.5,
												}}
											>
												Server Deafened
											</Typography>
										)}
										{user.self_mute && (
											<Typography
												variant="caption"
												sx={{
													bgcolor: "action.selected",
													px: 0.75,
													py: 0.25,
													borderRadius: 0.5,
												}}
											>
												Muted
											</Typography>
										)}
										{user.self_deaf && (
											<Typography
												variant="caption"
												sx={{
													bgcolor: "action.selected",
													px: 0.75,
													py: 0.25,
													borderRadius: 0.5,
												}}
											>
												Deafened
											</Typography>
										)}
										{user.self_stream && (
											<Typography
												variant="caption"
												sx={{
													bgcolor: "info.dark",
													px: 0.75,
													py: 0.25,
													borderRadius: 0.5,
												}}
											>
												Streaming
											</Typography>
										)}
									</Box>
								</CardContent>
							</Card>
						</Box>
					))}
				</Box>
			)}
		</>
	);
}
