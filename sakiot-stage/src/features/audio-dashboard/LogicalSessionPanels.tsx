import { ChevronDown as ExpandMoreIcon } from "lucide-react";
import type { ReactNode, RefObject } from "react";
import {
	Accordion,
	AccordionDetails,
	AccordionSummary,
	Alert,
	Box,
	Button,
	Chip,
	FormControl,
	InputLabel,
	MenuItem,
	Paper,
	Select,
	Stack,
	Typography,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import type { PlaybackSegment } from "./logicalSessionTimeline";

export function LogicalSessionSummary(props: {
	sessionId: string;
	state: string;
	userId: string;
	startedAtMs: number;
	durationMs: number;
	physicalCount: number;
	currentSegment?: PlaybackSegment;
}) {
	const current = props.currentSegment;
	return (
		<>
			<Stack direction="row" spacing={1} flexWrap="wrap" sx={{ mb: 2 }}>
				<Chip label={`Session ${props.sessionId}`} />
				<Chip
					label={props.state}
					color={props.state === "active" ? "error" : "default"}
				/>
				<Chip label={`User ${props.userId}`} />
				<Chip
					label={`${props.physicalCount} physical ${
						props.physicalCount === 1 ? "file" : "files"
					}`}
					variant="outlined"
				/>
				{current && (
					<Chip
						label={
							current.reason === "channel_filtered"
								? `Channel ${current.channel_id ?? "?"} muted`
								: current.kind === "silence"
									? `Silence · ${current.reason ?? "gap"}`
									: `Channel ${current.channel_id ?? "?"}`
						}
						color={
							current.reason === "channel_filtered"
								? "default"
								: current.kind === "silence"
									? "warning"
									: "primary"
						}
					/>
				)}
			</Stack>
			<Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
				Started {new Date(props.startedAtMs).toLocaleString()} · duration{" "}
				{formatDuration(props.durationMs / 1_000)}
			</Typography>
		</>
	);
}

export function PlaybackActionsPanel(props: { children: ReactNode }) {
	return (
		<Paper variant="outlined" sx={{ p: 2, my: 2 }}>
			{props.children}
		</Paper>
	);
}

export function SessionClipEditorPanel(props: {
	children: ReactNode;
	panelRef: RefObject<HTMLDivElement | null>;
}) {
	return (
		<Paper ref={props.panelRef} sx={{ p: 2, my: 2 }}>
			{props.children}
		</Paper>
	);
}

export function PhysicalRecordingsPanel(props: {
	sessionId: string;
	fragments: PlaybackSegment[];
	allFragments: PlaybackSegment[];
	channelIds: string[];
	effectiveChannelId: string | null;
	onSelectChannel: (channelId: string | null) => void;
	onSeek: (positionMs: number) => void;
}) {
	return (
		<Accordion variant="outlined" disableGutters sx={{ my: 2 }}>
			<AccordionSummary
				expandIcon={<ExpandMoreIcon />}
				aria-controls={`session-${props.sessionId}-physical-content`}
				id={`session-${props.sessionId}-physical-header`}
			>
				<Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap">
					<Typography>Physical recordings</Typography>
					<Chip
						size="small"
						variant="outlined"
						label={`${props.fragments.length} ${
							props.fragments.length === 1 ? "file" : "files"
						}`}
					/>
					{props.effectiveChannelId && (
						<Chip
							size="small"
							color="primary"
							label={`Channel ${props.effectiveChannelId} only`}
						/>
					)}
				</Stack>
			</AccordionSummary>
			<AccordionDetails id={`session-${props.sessionId}-physical-content`}>
				<Stack
					direction={{ xs: "column", md: "row" }}
					spacing={2}
					justifyContent="space-between"
					alignItems={{ xs: "stretch", md: "flex-start" }}
				>
					<Typography variant="body2" color="text.secondary">
						Session combines channel-bound files into one timestamp-aligned
						timeline.
					</Typography>
					{props.channelIds.length > 1 && (
						<FormControl size="small" sx={{ minWidth: 260 }}>
							<InputLabel id={`session-${props.sessionId}-channel-label`}>
								Playback channel
							</InputLabel>
							<Select
								labelId={`session-${props.sessionId}-channel-label`}
								label="Playback channel"
								value={props.effectiveChannelId ?? ""}
								onChange={(event) =>
									props.onSelectChannel(event.target.value || null)
								}
							>
								<MenuItem value="">All channels</MenuItem>
								{props.channelIds.map((channelId) => {
									const count = props.allFragments.filter(
										(fragment) => fragment.channel_id === channelId,
									).length;
									return (
										<MenuItem key={channelId} value={channelId}>
											Channel {channelId} · {count}{" "}
											{count === 1 ? "file" : "files"}
										</MenuItem>
									);
								})}
							</Select>
						</FormControl>
					)}
				</Stack>
				<Stack spacing={0.75} sx={{ mt: 1.5 }}>
					{props.fragments.map((fragment, index) => (
						<Button
							key={
								fragment.audio_file_id ??
								`${fragment.start_ms}-${fragment.end_ms}`
							}
							variant="text"
							onClick={() => props.onSeek(fragment.start_ms)}
							sx={{
								justifyContent: "flex-start",
								textTransform: "none",
								px: 1,
							}}
						>
							<Box sx={{ textAlign: "left" }}>
								<Typography variant="body2">
									Fragment {(fragment.segment_index ?? index) + 1} · Channel{" "}
									{fragment.channel_id ?? "?"} ·{" "}
									{formatDuration(fragment.start_ms / 1_000)} –{" "}
									{formatDuration(fragment.end_ms / 1_000)}
								</Typography>
								<Typography variant="caption" color="text.secondary">
									{fragment.file_name ?? `File ${fragment.audio_file_id}`}
								</Typography>
							</Box>
						</Button>
					))}
				</Stack>
				{props.effectiveChannelId && (
					<Alert severity="info" sx={{ mt: 1.5 }}>
						Only Channel {props.effectiveChannelId} plays. Other channels stay
						muted while timeline offsets remain unchanged. Downloads and clips
						still use full session.
					</Alert>
				)}
			</AccordionDetails>
		</Accordion>
	);
}
