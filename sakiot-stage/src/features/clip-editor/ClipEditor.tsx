import ContentCopyIcon from "@mui/icons-material/ContentCopy";
import FitScreenIcon from "@mui/icons-material/FitScreen";
import PauseIcon from "@mui/icons-material/Pause";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import RedoIcon from "@mui/icons-material/Redo";
import RepeatIcon from "@mui/icons-material/Repeat";
import UndoIcon from "@mui/icons-material/Undo";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import IconButton from "@mui/material/IconButton";
import Slider from "@mui/material/Slider";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import { useCallback, useEffect, useRef } from "react";
import { useSearchParams } from "react-router-dom";
import { type ClipData, useGetClipsQuery } from "../../app/apiSlice";
import { useAsRole } from "../../app/useAsRole";
import { formatDuration } from "../../utils/formatTime";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "../audio-dashboard/playbackShortcuts";
import { ClipBin } from "./ClipBin";
import { Inspector } from "./Inspector";
import { type ClipEdit, makeSegment } from "./model";
import { Timeline } from "./Timeline";
import { useClipBuffer } from "./useClipBuffer";
import { useClipEditor } from "./useClipEditor";

export function ClipEditor(props: { guildId: string }) {
	const { asRoleArg } = useAsRole();
	const { data: clips } = useGetClipsQuery(
		{ guild_id: props.guildId, ...asRoleArg },
		{ skip: !props.guildId },
	);
	const editor = useClipEditor();
	const [searchParams] = useSearchParams();
	const sourceClipId = searchParams.get("source");
	const seededRef = useRef(false);

	const clipName = useCallback(
		(sourceId: string) => {
			if (sourceId.startsWith("session")) return sourceId;
			return clips?.find((c) => c.clip_id === sourceId)?.name ?? sourceId;
		},
		[clips],
	);

	const { buffer: sourceBuffer, status: sourceStatus } = useClipBuffer(
		props.guildId,
		sourceClipId,
	);

	useEffect(() => {
		if (seededRef.current || !sourceClipId || !sourceBuffer) return;
		seededRef.current = true;
		editor.registerBuffer(sourceClipId, sourceBuffer);
		const lengthSec =
			clips?.find((c) => c.clip_id === sourceClipId)?.length ??
			sourceBuffer.duration;
		const segment = makeSegment("clip", sourceClipId, 0, lengthSec, 0, 0);
		editor.apply((edit) => ({
			...edit,
			segments: [...edit.segments, segment],
		}));
		editor.select(segment.id);
	}, [clips, editor, sourceBuffer, sourceClipId]);

	const handleDropClip = useCallback(
		(
			track: number,
			clientX: number,
			element: HTMLElement,
			dataTransfer: DataTransfer,
		) => {
			const clipId = dataTransfer.getData("text/plain");
			if (!clipId) return;
			const clip = clips?.find((c) => c.clip_id === clipId);
			if (!clip) return;
			const bounds = element.getBoundingClientRect();
			const fraction = Math.min(
				1,
				Math.max(0, (clientX - bounds.left) / Math.max(1, bounds.width)),
			);
			let startSec = editor.viewStartSec + fraction * editor.viewWidthSec;
			if (Math.abs(startSec - editor.positionSec) < 0.25) {
				startSec = editor.positionSec;
			}
			editor.loadClip(
				props.guildId,
				clip.clip_id,
				clip.length ?? 1,
				track,
				Math.max(0, startSec),
			);
		},
		[clips, editor, props.guildId],
	);

	const handleAddFromBin = useCallback(
		(clip: ClipData) => {
			editor.loadClip(props.guildId, clip.clip_id, clip.length ?? 1, 0);
		},
		[editor, props.guildId],
	);

	useKeyboardShortcuts(editor);

	const duration = editor.edit.segments.reduce(
		(max, segment) =>
			Math.max(
				max,
				segment.timelineStart + (segment.sourceOut - segment.sourceIn),
			),
		0,
	);

	return (
		<Box
			sx={{
				height: "100%",
				minHeight: 0,
				display: "flex",
				flexDirection: "column",
			}}
		>
			<ToolbarRow editor={editor} />
			<Box
				sx={{
					flex: 1,
					minHeight: 0,
					minWidth: 0,
					display: "flex",
				}}
			>
				<ClipBin
					clips={clips ?? []}
					loadingClips={editor.loadingClips}
					onAdd={handleAddFromBin}
				/>
				<Box
					sx={{
						flex: 1,
						minWidth: 0,
						display: "flex",
						flexDirection: "column",
					}}
				>
					<Monitor
						editor={editor}
						duration={duration}
						sourceStatus={sourceStatus}
					/>
					<Box sx={{ flex: 1, minHeight: 0 }}>
						<Timeline
							editor={editor}
							clipName={(segment) => clipName(segment.sourceId)}
							onDropClip={handleDropClip}
						/>
					</Box>
				</Box>
				<Inspector editor={editor} clipName={clipName} />
			</Box>
		</Box>
	);
}

function ToolbarRow(props: { editor: ReturnType<typeof useClipEditor> }) {
	const { editor } = props;
	return (
		<Box
			sx={{
				display: "flex",
				alignItems: "center",
				gap: 1,
				px: 2,
				py: 1,
				borderBottom: 1,
				borderColor: "divider",
			}}
		>
			<Typography variant="h6" sx={{ flex: 1, minWidth: 0 }} noWrap>
				Clip Editor
			</Typography>
			<Tooltip title="Undo (Ctrl+Z)">
				<span>
					<IconButton
						size="small"
						disabled={!editor.canUndo}
						onClick={editor.undo}
					>
						<UndoIcon fontSize="small" />
					</IconButton>
				</span>
			</Tooltip>
			<Tooltip title="Redo (Ctrl+Shift+Z)">
				<span>
					<IconButton
						size="small"
						disabled={!editor.canRedo}
						onClick={editor.redo}
					>
						<RedoIcon fontSize="small" />
					</IconButton>
				</span>
			</Tooltip>
			<Tooltip title="Add track">
				<Button
					size="small"
					variant="outlined"
					onClick={() =>
						editor.apply((edit) => ({ ...edit, tracks: edit.tracks + 1 }))
					}
				>
					+ Track
				</Button>
			</Tooltip>
			<Tooltip title="Fit edit in view">
				<IconButton size="small" onClick={editor.fitView}>
					<FitScreenIcon fontSize="small" />
				</IconButton>
			</Tooltip>
			<Tooltip title="Export (coming soon)">
				<span>
					<Button
						size="small"
						variant="contained"
						startIcon={<ContentCopyIcon />}
						disabled
					>
						Export
					</Button>
				</span>
			</Tooltip>
		</Box>
	);
}

function Monitor(props: {
	editor: ReturnType<typeof useClipEditor>;
	duration: number;
	sourceStatus: "idle" | "loading" | "ready" | "error";
}) {
	const { editor } = props;

	return (
		<Box
			sx={{
				display: "flex",
				alignItems: "center",
				gap: 2,
				px: 2,
				py: 1,
				borderBottom: 1,
				borderColor: "divider",
				flexWrap: "wrap",
			}}
		>
			<Button
				variant="contained"
				startIcon={editor.playing ? <PauseIcon /> : <PlayArrowIcon />}
				onClick={editor.togglePlay}
				disabled={props.duration <= 0}
			>
				{editor.playing ? "Pause" : "Play"}
			</Button>
			<Tooltip title="Loop the edit while playing">
				<IconButton
					size="small"
					color={editor.loop ? "secondary" : "default"}
					aria-pressed={editor.loop}
					onClick={() => editor.setLooping(!editor.loop)}
				>
					<RepeatIcon fontSize="small" />
				</IconButton>
			</Tooltip>
			<Typography variant="body2" sx={{ fontVariantNumeric: "tabular-nums" }}>
				{formatDuration(editor.positionSec)} / {formatDuration(props.duration)}
			</Typography>
			<Box sx={{ minWidth: 160, width: 200 }}>
				<Typography variant="caption" color="text.secondary">
					Master volume {editor.masterVolumeDb.toFixed(1)} dB
				</Typography>
				<Slider
					size="small"
					min={-40}
					max={12}
					step={0.5}
					value={editor.masterVolumeDb}
					onChange={(_event, value) => editor.setMasterVolume(Number(value))}
					aria-label="Master volume"
					sx={{
						"& .MuiSlider-thumb, & .MuiSlider-track": {
							transition: "none",
						},
					}}
				/>
			</Box>
			<Chip
				label={`${editor.edit.segments.length} segment${editor.edit.segments.length === 1 ? "" : "s"}`}
				size="small"
			/>
			{props.sourceStatus === "loading" && (
				<Chip label="Loading source…" size="small" color="warning" />
			)}
			{props.sourceStatus === "error" && (
				<Alert severity="error" sx={{ py: 0 }}>
					Source clip could not be loaded.
				</Alert>
			)}
		</Box>
	);
}

function useKeyboardShortcuts(editor: ReturnType<typeof useClipEditor>) {
	const editorRef = useRef(editor);
	useEffect(() => {
		editorRef.current = editor;
	}, [editor]);

	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			const current = editorRef.current;
			if (playbackShortcutTargetAcceptsText(event.target)) return;
			const modifier = event.ctrlKey || event.metaKey;
			if (modifier && event.key.toLowerCase() === "z") {
				event.preventDefault();
				if (event.shiftKey) current.redo();
				else current.undo();
				return;
			}
			if (modifier && event.key.toLowerCase() === "y") {
				event.preventDefault();
				current.redo();
				return;
			}
			if (modifier) return;
			if (event.key === " " || event.code === "Space") {
				if (event.repeat) return;
				event.preventDefault();
				current.togglePlay();
				return;
			}
			const segment = current.selectedSegment;
			if (event.key === "Delete" || event.key === "Backspace") {
				if (segment) {
					event.preventDefault();
					current.removeSelected();
				}
				return;
			}
			if (!segment) return;
			const at = current.positionSec - segment.timelineStart;
			if (event.key === "i" || event.key === "I") {
				event.preventDefault();
				trimSegment(current, segment.id, "in", at);
				return;
			}
			if (event.key === "o" || event.key === "O") {
				event.preventDefault();
				trimSegment(current, segment.id, "out", at);
				return;
			}
			if (event.key === "s" || event.key === "S") {
				event.preventDefault();
				current.splitSelectedAtPlayhead();
				return;
			}
			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			const delta =
				(event.shiftKey ? 1 : 0.1) * (event.key === "ArrowRight" ? 1 : -1);
			event.preventDefault();
			current.apply((edit) => moveSegmentBy(edit, segment.id, delta));
		};
		window.addEventListener("keydown", handleShortcut);
		return () => window.removeEventListener("keydown", handleShortcut);
	}, []);
}

function trimSegment(
	editor: ReturnType<typeof useClipEditor>,
	id: string,
	edge: "in" | "out",
	atSec: number,
) {
	editor.apply((edit) => ({
		...edit,
		segments: edit.segments.map((s) => {
			if (s.id !== id) return s;
			const sourceIn = edge === "in" ? Math.max(0, atSec) : s.sourceIn;
			const sourceOut =
				edge === "out" ? Math.max(sourceIn + 0.05, atSec) : s.sourceOut;
			return { ...s, sourceIn, sourceOut };
		}),
	}));
}

function moveSegmentBy(edit: ClipEdit, id: string, delta: number): ClipEdit {
	return {
		...edit,
		segments: edit.segments.map((s) =>
			s.id === id
				? { ...s, timelineStart: Math.max(0, s.timelineStart + delta) }
				: s,
		),
	};
}
