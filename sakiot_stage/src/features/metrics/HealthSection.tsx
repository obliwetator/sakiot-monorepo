import Box from "@mui/material/Box";
import { GroupPanel, Tile } from "../../shared/primitives";
import { formatBytes, formatTimeSince } from "../../utils/formatTime";
import type { Metrics } from "./types";

export function HealthSection({
	m,
	currentTime,
	L,
}: {
	m: Metrics | null;
	currentTime: number;
	L: string;
}) {
	return (
		<Box
			sx={{
				display: "flex",
				gap: 3,
				flexWrap: "wrap",
				alignItems: "flex-start",
			}}
		>
			<Box sx={{ flex: "1 1 340px" }}>
				<GroupPanel title="Voice Recording Pipeline">
					<Tile label="Active Recordings" value={m ? m.active_recordings : L} />
					<Tile
						label="Packets Received"
						value={m ? m.audio_packets_received.toLocaleString() : L}
					/>
					<Tile
						label="Packets Dropped"
						value={m ? m.audio_packets_dropped.toLocaleString() : L}
						warn={!!m && m.audio_packets_dropped > 0}
					/>
					<Tile
						label="FFmpeg Spawn Failures"
						value={m ? m.ffmpeg_spawn_failures : L}
						warn={!!m && m.ffmpeg_spawn_failures > 0}
					/>
					<Tile
						label="FFmpeg Crashes"
						value={m ? m.ffmpeg_process_crashes : L}
						warn={!!m && m.ffmpeg_process_crashes > 0}
					/>
					<Tile
						label="Last Voice Packet"
						value={
							m ? formatTimeSince(m.last_voice_packet_time, currentTime) : L
						}
					/>
				</GroupPanel>
			</Box>

			<Box
				sx={{
					flex: "1 1 340px",
					display: "flex",
					flexDirection: "column",
					gap: 2,
				}}
			>
				<GroupPanel title="Discord Gateway">
					<Tile
						label="Voice State Updates"
						value={m ? m.voice_state_updates_received.toLocaleString() : L}
					/>
					<Tile
						label="Gateway Reconnects"
						value={m ? m.gateway_reconnects : L}
						warn={!!m && m.gateway_reconnects > 0}
					/>
					<Tile
						label="Driver Reconnects"
						value={m ? m.driver_reconnects : L}
						warn={!!m && m.driver_reconnects > 0}
					/>
				</GroupPanel>

				<GroupPanel title="Database & gRPC">
					<Tile
						label="DB Query Errors"
						value={m ? m.db_query_errors : L}
						warn={!!m && m.db_query_errors > 0}
					/>
					<Tile
						label="DB Insert Failures"
						value={m ? m.db_insert_failures : L}
						warn={!!m && m.db_insert_failures > 0}
					/>
					<Tile
						label="Active gRPC Streams"
						value={m ? m.grpc_active_streams : L}
					/>
				</GroupPanel>

				<GroupPanel title="Process">
					<Tile
						label="Memory (RSS)"
						value={m ? formatBytes(m.process_rss_bytes) : L}
					/>
					<Tile label="Open FDs" value={m ? m.process_open_fds : L} />
					<Tile label="Tokio Tasks" value={m ? m.tokio_active_tasks : L} />
				</GroupPanel>
			</Box>
		</Box>
	);
}
