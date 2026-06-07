import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import { StatCard } from "../../shared/primitives";
import { formatUptime } from "../../utils/formatTime";
import type { Metrics } from "./types";

export function Overview({
	m,
	localUptime,
	L,
}: {
	m: Metrics | null;
	localUptime: number;
	L: string;
}) {
	return (
		<>
			<Typography variant="overline" color="text.secondary" fontWeight={700}>
				Overview
			</Typography>
			<Box sx={{ display: "flex", flexWrap: "wrap", gap: 2, mt: 1, mb: 3 }}>
				<StatCard label="Total Guilds" value={m ? m.total_guilds : L} />
				<StatCard
					label="Active Voice Connections"
					value={m ? m.active_voice_connections : L}
				/>
				<StatCard
					label="Commands Executed"
					value={m ? m.commands_executed : L}
				/>
				<StatCard
					label="Messages Received"
					value={m ? m.messages_received.toLocaleString() : L}
				/>
				<StatCard label="Uptime" value={m ? formatUptime(localUptime) : L} />
			</Box>
		</>
	);
}
