import Autocomplete from "@mui/material/Autocomplete";
import Box from "@mui/material/Box";
import Divider from "@mui/material/Divider";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import { useState } from "react";
import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { HealthSection } from "./HealthSection";
import { useGuildVoiceStream, useMetricsStream, useNowTick } from "./hooks";
import { Overview } from "./Overview";
import type { GuildInfo } from "./types";
import { VoiceUsersSection } from "./VoiceUsersSection";

export function Metrics() {
	const [selectedGuild, setSelectedGuild] = useState<GuildInfo | null>(null);

	useGetAuthDetailsQuery(undefined, { skip: !hasLoggedInCookie() });
	const isAdmin = true;

	const { metrics: m, localUptime, refreshCounter } = useMetricsStream(isAdmin);
	const { voiceUsers, userStartTimes, guildRecordingMetrics } =
		useGuildVoiceStream(isAdmin, selectedGuild?.id ?? null, refreshCounter);
	const currentTime = useNowTick();

	if (!isAdmin) {
		return (
			<Box p={2}>
				<Typography color="error">Not authorized.</Typography>
			</Box>
		);
	}

	const LoadingLabel = "Loading...";

	return (
		<Box sx={{ flexGrow: 1, p: { xs: 1.5, md: 3 }, maxWidth: 1400 }}>
			<Box
				sx={{
					display: "flex",
					alignItems: "center",
					justifyContent: "space-between",
					mb: 3,
					flexWrap: "wrap",
					gap: 2,
				}}
			>
				<Typography variant="h4" fontWeight={700}>
					Bot Metrics Dashboard
				</Typography>
				<Box sx={{ width: { xs: "100%", sm: 280 } }}>
					<Autocomplete
						size="small"
						options={m?.guilds || []}
						getOptionLabel={(o) => o.name}
						value={selectedGuild}
						onChange={(_, v) => setSelectedGuild(v)}
						isOptionEqualToValue={(o, v) => o.id === v.id}
						renderInput={(params) => <TextField {...params} label="Guild" />}
					/>
				</Box>
			</Box>

			<Overview m={m} localUptime={localUptime} L={LoadingLabel} />

			<Divider sx={{ mb: 3 }} />

			<HealthSection m={m} currentTime={currentTime} L={LoadingLabel} />

			<Divider sx={{ my: 3 }} />
			<Typography variant="h5" fontWeight={600} gutterBottom>
				Active Voice Users {selectedGuild ? `— ${selectedGuild.name}` : ""}
			</Typography>

			<VoiceUsersSection
				selectedGuild={selectedGuild}
				guildRecordingMetrics={guildRecordingMetrics}
				voiceUsers={voiceUsers}
				userStartTimes={userStartTimes}
				currentTime={currentTime}
			/>
		</Box>
	);
}
