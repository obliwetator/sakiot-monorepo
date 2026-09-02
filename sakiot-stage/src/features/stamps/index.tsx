import { useSelector } from "react-redux";
import { useNavigate, useParams } from "react-router-dom";
import { useGetStampsQuery } from "../../app/apiSlice";
import { useAsRole } from "../../app/useAsRole";
import {
	Button,
	Notice,
	Page,
	PageTitle,
	Table,
	TableBody,
	TableCell,
	TableContainer,
	TableHead,
	TableHeader,
	TableRow,
	Text,
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
			<Page>
				<Notice tone="info">
					Select a guild from the top navbar to view stamps.
				</Notice>
			</Page>
		);
	}

	if (isLoading) {
		return (
			<Page>
				<Text aria-live="polite">Loading stamps…</Text>
			</Page>
		);
	}

	if (isError) {
		return (
			<Page>
				<Notice tone="error" announce="alert">
					Failed to load stamps: {JSON.stringify(error)}
				</Notice>
			</Page>
		);
	}

	const rows = data ?? [];

	return (
		<Page className="max-w-7xl space-y-5">
			<ViewAsRoleBanner guildId={guildId} />
			<header className="space-y-1">
				<PageTitle>Stamps {guildName ? `— ${guildName}` : ""}</PageTitle>
				<Text tone="muted">
					{rows.length} stamp{rows.length === 1 ? "" : "s"} (newest first, max
					500). Session links open the complete logical recording. Audio file
					IDs identify the physical fragment containing each stamp.
				</Text>
			</header>

			{rows.length === 0 ? (
				<Notice tone="info">No stamps yet.</Notice>
			) : (
				<TableContainer className="rounded-lg border border-slate-800">
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
									<TableRow key={s.id} hover>
										<TableCell>{s.id}</TableCell>
										<TableCell>{formatTimestamp(s.stamp_ts)}</TableCell>
										<TableCell>
											{playbackTarget ? (
												<Button
													variant="secondary"
													size="sm"
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
												<span className="opacity-50">—</span>
											)}
										</TableCell>
										<TableCell>
											<div>{s.target_name ?? s.target_user_id}</div>
											{s.target_name && (
												<div className="text-[11px] opacity-60">
													{s.target_user_id}
												</div>
											)}
										</TableCell>
										<TableCell>
											<div>{s.stamper_name ?? s.stamper_user_id}</div>
											{s.stamper_name && (
												<div className="text-[11px] opacity-60">
													{s.stamper_user_id}
												</div>
											)}
										</TableCell>
										<TableCell>
											<div>{s.channel_name ?? s.channel_id}</div>
											{s.channel_name && (
												<div className="text-[11px] opacity-60">
													{s.channel_id}
												</div>
											)}
										</TableCell>
										<TableCell align="right">{s.offset_ms}</TableCell>
										<TableCell>
											{sessionPath ? (
												<Button
													variant="secondary"
													size="sm"
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
												<span className="opacity-50">—</span>
											)}
										</TableCell>
										<TableCell>
											{s.audio_file_id ? (
												<div>
													<div className="text-sm font-medium">
														{s.audio_file_id}
													</div>
													<div className="text-xs text-muted">
														{s.recording_session_id
															? `Fragment ${fragmentNumber ?? "?"}${
																	s.session_fragment_count
																		? ` of ${s.session_fragment_count}`
																		: ""
																}`
															: "Legacy physical file"}
													</div>
												</div>
											) : (
												<span className="opacity-50">—</span>
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
		</Page>
	);
}
