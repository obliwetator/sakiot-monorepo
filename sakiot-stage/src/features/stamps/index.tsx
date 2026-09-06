import { useSelector } from "react-redux";
import { useNavigate, useParams } from "react-router-dom";
import { useGetStampsQuery } from "../../app/apiSlice";
import { useAsRole } from "../../app/useAsRole";
import {
	Button,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tooltip,
	TooltipTrigger,
} from "../../shared/ui";
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
			<div className="p-6">
				<p className="leading-6 text-muted">
					Select a guild from the top navbar to view stamps.
				</p>
			</div>
		);
	}

	if (isLoading) {
		return (
			<div className="p-6">
				<p className="leading-6">Loading stamps…</p>
			</div>
		);
	}

	if (isError) {
		return (
			<div className="p-6">
				<p className="leading-6 text-danger">
					Failed to load stamps: {JSON.stringify(error)}
				</p>
			</div>
		);
	}

	const rows = data ?? [];

	return (
		<div className="p-3 min-[900px]:p-6 max-w-350">
			<ViewAsRoleBanner guildId={guildId} />
			<h4 className="leading-6 [font-weight:700] font-semibold tracking-tight mb-2">
				Stamps {guildName ? `— ${guildName}` : ""}
			</h4>
			<p className="leading-6 text-muted mb-4">
				{rows.length} stamp{rows.length === 1 ? "" : "s"} (newest first, max
				500). Session links open the complete logical recording. Audio file IDs
				identify the physical fragment containing each stamp.
			</p>

			{rows.length === 0 ? (
				<p className="leading-6 text-muted">No stamps yet.</p>
			) : (
				<div className="w-full overflow-x-auto">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>ID</TableHead>
								<TableHead>Absolute Time</TableHead>
								<TableHead>Relative Time</TableHead>
								<TableHead>Target</TableHead>
								<TableHead>Stamper</TableHead>
								<TableHead>Channel</TableHead>
								<TableHead align="right">Offset (ms)</TableHead>
								<TableHead>Session ID</TableHead>
								<TableHead>Audio File ID</TableHead>
								<TableHead>Note</TableHead>
								<TableHead>Created</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{rows.map((s) => {
								const playbackTarget = buildStampPlaybackTarget(s, guildId);
								const fragmentNumber =
									s.segment_index == null ? null : s.segment_index + 1;
								const sessionPath = s.recording_session_id
									? `/dashboard/${encodeURIComponent(guildId)}/audio/session/${encodeURIComponent(s.recording_session_id)}`
									: null;
								return (
									<TableRow key={s.id}>
										<TableCell>{s.id}</TableCell>
										<TableCell>{formatTimestamp(s.stamp_ts)}</TableCell>
										<TableCell>
											{playbackTarget ? (
												<TooltipTrigger delay={400}>
													<Button
														variant="ghost"
														size="sm"
														onPress={() => navigate(playbackTarget.path)}
													>
														{formatDuration(playbackTarget.relativeSeconds)}
													</Button>
													<Tooltip>
														{playbackTarget.scope === "session"
															? "Open complete logical session"
															: "Open legacy physical fragment"}
													</Tooltip>
												</TooltipTrigger>
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
													variant="ghost"
													size="sm"
													onPress={() =>
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
												<div>
													<p className="text-sm">{s.audio_file_id}</p>
													<span className="text-muted block text-xs leading-5">
														{s.recording_session_id
															? `Fragment ${fragmentNumber ?? "?"}${
																	s.session_fragment_count
																		? ` of ${s.session_fragment_count}`
																		: ""
																}`
															: "Legacy physical file"}
													</span>
												</div>
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
				</div>
			)}
		</div>
	);
}
