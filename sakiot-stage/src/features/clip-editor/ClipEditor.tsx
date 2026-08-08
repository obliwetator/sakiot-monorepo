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
import LinearProgress from "@mui/material/LinearProgress";
import Slider from "@mui/material/Slider";
import TextField from "@mui/material/TextField";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import {
	apiSlice,
	type ClipData,
	useComposeClipMutation,
	useGetClipsQuery,
	useGetComposeClipStatusQuery,
} from "../../app/apiSlice";
import { useAppDispatch } from "../../app/hooks";
import { useAsRole } from "../../app/useAsRole";
import { BaseDialog } from "../../shared/BaseDialog";
import { formatDuration } from "../../utils/formatTime";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "../audio-dashboard/playbackShortcuts";
import { isComposedClip } from "../clips/composedClip";
import { ClipBin } from "./ClipBin";
import { deserializeEdit, serializeEdit } from "./composePayload";
import { Inspector } from "./Inspector";
import { type ClipEdit, makeSegment, segmentDuration } from "./model";
import { Timeline } from "./Timeline";
import { useClipBuffer } from "./useClipBuffer";
import { useClipEditor } from "./useClipEditor";

export function ClipEditor(props: { guildId: string }) {
	const { asRoleArg } = useAsRole();
	const dispatch = useAppDispatch();
	const { data: clips } = useGetClipsQuery(
		{ guild_id: props.guildId, ...asRoleArg },
		{ skip: !props.guildId },
	);
	const editor = useClipEditor();
	const [searchParams] = useSearchParams();
	const sourceClipId = searchParams.get("source");
	const seededRef = useRef(false);

	const [composeOpen, setComposeOpen] = useState(false);
	const [composeName, setComposeName] = useState("");
	const [composeError, setComposeError] = useState<string | null>(null);
	const [composeDone, setComposeDone] = useState(false);
	const [composeId, setComposeId] = useState<string | null>(null);
	const [composeClip, { isLoading: composeStarting }] =
		useComposeClipMutation();
	const { data: composeStatus } = useGetComposeClipStatusQuery(
		{ guild_id: props.guildId, clip_id: composeId ?? "" },
		{ skip: composeId === null, pollingInterval: 1000 },
	);

	useEffect(() => {
		if (!composeId || !composeStatus) return;
		if (composeStatus.status === "ready") {
			dispatch(apiSlice.util.invalidateTags(["Clips"]));
			setComposeDone(true);
			setComposeId(null);
		} else if (composeStatus.status === "failed") {
			setComposeError(
				"The render failed. Check the source clips and try again.",
			);
			setComposeId(null);
		}
	}, [composeId, composeStatus, dispatch]);

	const handleCompose = useCallback(async () => {
		if (editor.edit.segments.length === 0) return;
		editor.flush();
		setComposeError(null);
		setComposeDone(false);
		try {
			const result = await composeClip({
				guild_id: props.guildId,
				body: serializeEdit(editor.edit, composeName.trim() || undefined),
			}).unwrap();
			setComposeId(result.id);
		} catch {
			setComposeError("Could not start the render. Please try again.");
		}
	}, [composeClip, composeName, editor, props.guildId]);

	const closeCompose = useCallback(() => {
		if (composeStarting || composeId !== null) return;
		setComposeOpen(false);
		setComposeError(null);
		setComposeDone(false);
	}, [composeId, composeStarting]);

	const clipName = useCallback(
		(sourceId: string) => {
			if (sourceId.startsWith("session")) return sourceId;
			return clips?.find((c) => c.clip_id === sourceId)?.name ?? sourceId;
		},
		[clips],
	);

	// Single selection rule: any pointerdown that is not on a segment (their
	// own handlers stop propagation) and not inside the inspector (which edits
	// the selection) clears the selection.
	useEffect(() => {
		const onGlobalPointerDown = (event: PointerEvent) => {
			const target = event.target as HTMLElement | null;
			if (target?.closest("[data-clip-editor-inspector]")) return;
			editor.select(null);
		};
		window.addEventListener("pointerdown", onGlobalPointerDown);
		return () => window.removeEventListener("pointerdown", onGlobalPointerDown);
	}, [editor.select]);

	const { buffer: sourceBuffer, status: sourceStatus } = useClipBuffer(
		props.guildId,
		sourceClipId,
	);

	useEffect(() => {
		if (seededRef.current || !sourceClipId) return;
		const source = clips?.find((c) => c.clip_id === sourceClipId);
		const edit = source?.composition
			? deserializeEdit(source.composition)
			: null;
		if (edit && edit.segments.length > 0) {
			// Composed clip: restore the whole edit, then load every source
			// buffer so the timeline plays as it did when it was exported.
			seededRef.current = true;
			editor.apply(() => edit);
			editor.select(edit.segments[0]?.id ?? null);
			void editor.preloadSources(
				props.guildId,
				edit.segments.map((segment) => segment.sourceId),
			);
			return;
		}
		if (!sourceBuffer) return;
		seededRef.current = true;
		editor.registerBuffer(sourceClipId, sourceBuffer);
		const lengthSec = source?.length ?? sourceBuffer.duration;
		const segment = makeSegment("clip", sourceClipId, 0, lengthSec, 0, 0);
		editor.apply((edit) => ({
			...edit,
			segments: [...edit.segments, segment],
		}));
		editor.select(segment.id);
	}, [clips, editor, props.guildId, sourceBuffer, sourceClipId]);

	const handleDropClip = useCallback(
		(clipId: string, lengthSec: number, track: number, startSec: number) => {
			editor.loadClip(props.guildId, clipId, lengthSec, track, startSec);
		},
		[editor, props.guildId],
	);

	const handleAddFromBin = useCallback(
		(clip: ClipData) => {
			editor.loadClip(props.guildId, clip.clip_id, clip.length ?? 1, 0);
		},
		[editor, props.guildId],
	);

	useKeyboardShortcuts(editor);

	const pureClips = (clips ?? []).filter((clip) => !isComposedClip(clip));

	const duration = editor.edit.segments.reduce(
		(max, segment) =>
			Math.max(max, segment.timelineStart + segmentDuration(segment)),
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
			<ToolbarRow
				editor={editor}
				onExport={() => setComposeOpen(true)}
				canExport={editor.edit.segments.length > 0}
			/>
			<Box
				sx={{
					flex: 1,
					minHeight: 0,
					minWidth: 0,
					display: "flex",
				}}
			>
				<ClipBin
					clips={pureClips}
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
							guildId={props.guildId}
							editor={editor}
							clipName={(segment) => clipName(segment.sourceId)}
							onDropClip={handleDropClip}
						/>
					</Box>
				</Box>
				<Inspector editor={editor} clipName={clipName} />
			</Box>
			<ClipExportDialog
				open={composeOpen}
				name={composeName}
				setName={setComposeName}
				error={composeError}
				isStarting={composeStarting}
				isRendering={composeId !== null}
				progress={composeStatus?.progress ?? 0}
				done={composeDone}
				segmentCount={editor.edit.segments.length}
				onStart={() => void handleCompose()}
				onClose={closeCompose}
			/>
		</Box>
	);
}

function ToolbarRow(props: {
	editor: ReturnType<typeof useClipEditor>;
	onExport: () => void;
	canExport: boolean;
}) {
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
			<Tooltip title="Export the composition as a new clip">
				<span>
					<Button
						size="small"
						variant="contained"
						startIcon={<ContentCopyIcon />}
						disabled={!props.canExport}
						onClick={props.onExport}
					>
						Export
					</Button>
				</span>
			</Tooltip>
		</Box>
	);
}

function ClipExportDialog(props: {
	open: boolean;
	name: string;
	setName: (name: string) => void;
	error: string | null;
	isStarting: boolean;
	isRendering: boolean;
	progress: number;
	done: boolean;
	segmentCount: number;
	onStart: () => void;
	onClose: () => void;
}) {
	const busy = props.isStarting || props.isRendering;
	return (
		<BaseDialog
			open={props.open}
			onClose={props.onClose}
			title="Export composition"
			error={props.error ?? undefined}
			busy={busy}
			actions={
				<>
					<Button onClick={props.onClose} disabled={busy}>
						{props.done ? "Done" : "Cancel"}
					</Button>
					{!props.done && (
						<Button
							variant="contained"
							disabled={busy || props.segmentCount === 0}
							onClick={props.onStart}
						>
							Render
						</Button>
					)}
				</>
			}
		>
			<TextField
				size="small"
				fullWidth
				label="Clip name"
				value={props.name}
				disabled={busy}
				onChange={(event) => props.setName(event.currentTarget.value)}
				sx={{ mb: 2 }}
			/>
			{props.done ? (
				<Typography variant="body2">
					Exported — the new clip is now in the bin.
				</Typography>
			) : props.isRendering ? (
				<Box>
					<Typography variant="body2" sx={{ mb: 1 }}>
						Rendering {props.segmentCount} segment
						{props.segmentCount === 1 ? "" : "s"} on the server…
					</Typography>
					<LinearProgress variant="determinate" value={props.progress} />
					<Typography
						variant="caption"
						color="text.secondary"
						sx={{ fontVariantNumeric: "tabular-nums" }}
					>
						{props.progress}%
					</Typography>
				</Box>
			) : (
				<Typography variant="body2" color="text.secondary">
					Renders {props.segmentCount} segment
					{props.segmentCount === 1 ? "" : "s"} into a single new clip.
				</Typography>
			)}
		</BaseDialog>
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
			if (segment) {
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
			}
			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			const direction = event.key === "ArrowRight" ? 1 : -1;
			const distance = event.shiftKey ? 1 : 0.1;
			event.preventDefault();
			if (segment) {
				current.apply((edit) =>
					moveSegmentBy(edit, segment.id, distance * direction),
				);
			} else {
				current.setPosition(
					Math.max(0, current.positionSec + distance * direction),
				);
			}
			return;
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
			const atContent = atSec * s.effects.rate;
			const sourceIn = edge === "in" ? Math.max(0, atContent) : s.sourceIn;
			const sourceOut =
				edge === "out" ? Math.max(sourceIn + 0.05, atContent) : s.sourceOut;
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
