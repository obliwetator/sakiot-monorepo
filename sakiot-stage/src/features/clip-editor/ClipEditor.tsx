import ContentCopyIcon from "@mui/icons-material/ContentCopy";
import FitScreenIcon from "@mui/icons-material/FitScreen";
import PauseIcon from "@mui/icons-material/Pause";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import RedoIcon from "@mui/icons-material/Redo";
import RepeatIcon from "@mui/icons-material/Repeat";
import RestoreIcon from "@mui/icons-material/Restore";
import UndoIcon from "@mui/icons-material/Undo";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import FormControl from "@mui/material/FormControl";
import FormControlLabel from "@mui/material/FormControlLabel";
import IconButton from "@mui/material/IconButton";
import LinearProgress from "@mui/material/LinearProgress";
import Radio from "@mui/material/Radio";
import RadioGroup from "@mui/material/RadioGroup";
import Slider from "@mui/material/Slider";
import Snackbar from "@mui/material/Snackbar";
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
import { loadDraft, saveDraft } from "./draftStorage";
import { EffectLimitsDialog } from "./EffectLimitsDialog";
import { EffectSettingsJsonDialog } from "./EffectSettingsJsonDialog";
import {
	type EffectLimits,
	loadEffectLimits,
	saveEffectLimits,
} from "./effectLimits";
import { isEffectSettingsJsonShortcut } from "./effectSettingsJson";
import { Inspector } from "./Inspector";
import { isInspectorFeatureDisabled } from "./inspectorFeaturePolicy";
import { addSegment, emptyEdit, makeSegment, segmentDuration } from "./model";
import { Timeline } from "./Timeline";
import { useUnsavedChangesGuard } from "./unsavedChangesGuard";
import { useClipBuffer } from "./useClipBuffer";
import { useClipEditor } from "./useClipEditor";

export function ClipEditor(props: { guildId: string }) {
	const { asRoleArg } = useAsRole();
	const dispatch = useAppDispatch();
	const { data: clips, isError: clipsError } = useGetClipsQuery(
		{ guild_id: props.guildId, ...asRoleArg },
		{ skip: !props.guildId },
	);
	const editor = useClipEditor();
	// Any undoable or redoable history step means the page holds work.
	const { dialog: unsavedDialog } = useUnsavedChangesGuard(
		editor.canUndo || editor.canRedo,
	);
	const [searchParams] = useSearchParams();
	const sourceClipId = searchParams.get("source");
	const seededForRef = useRef<string | null>(null);

	const [composeOpen, setComposeOpen] = useState(false);
	const [effectSettingsJsonOpen, setEffectSettingsJsonOpen] = useState(false);
	const [effectLimitsOpen, setEffectLimitsOpen] = useState(false);
	const [effectLimits, setEffectLimits] = useState<EffectLimits>(() =>
		loadEffectLimits(),
	);
	const [composeName, setComposeName] = useState("");
	const [composeError, setComposeError] = useState<string | null>(null);
	const [composeDone, setComposeDone] = useState(false);
	const [composeId, setComposeId] = useState<string | null>(null);
	const [overwrite, setOverwrite] = useState(false);
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
				body: serializeEdit(
					editor.edit,
					composeName.trim() || undefined,
					overwrite ? (sourceClipId ?? undefined) : undefined,
					effectLimits,
				),
			}).unwrap();
			setComposeId(result.id);
		} catch {
			setComposeError("Could not start the render. Please try again.");
		}
	}, [
		composeClip,
		composeName,
		editor,
		effectLimits,
		overwrite,
		props.guildId,
		sourceClipId,
	]);

	const updateEffectLimits = useCallback((limits: EffectLimits) => {
		setEffectLimits(limits);
		saveEffectLimits(limits);
	}, []);

	const closeCompose = useCallback(() => {
		if (composeStarting || composeId !== null) return;
		setComposeOpen(false);
		setComposeError(null);
		setComposeDone(false);
		setOverwrite(false);
	}, [composeId, composeStarting]);

	const clipName = useCallback(
		(sourceId: string) => {
			if (sourceId.startsWith("session")) return sourceId;
			return clips?.find((c) => c.clip_id === sourceId)?.name ?? sourceId;
		},
		[clips],
	);

	// The selection sticks: nothing else on the page clears it. Escape (see
	// useKeyboardShortcuts) is the only way to deselect without picking
	// another segment.

	const {
		buffer: sourceBuffer,
		status: sourceStatus,
		error: sourceError,
	} = useClipBuffer(props.guildId, sourceClipId);

	// The working draft lives in localStorage per session context (the source
	// clip it was seeded from, or one generic draft). Restore it before the
	// seed effect so a refresh comes back to the last saved state; marking the
	// source as seeded keeps the database payload from overwriting the draft.
	const draftRestoredRef = useRef(false);
	useEffect(() => {
		if (draftRestoredRef.current) return;
		draftRestoredRef.current = true;
		const draft = loadDraft(props.guildId, sourceClipId);
		if (!draft || draft.segments.length === 0) return;
		editor.reset(draft);
		editor.select(draft.segments[0]?.id ?? null);
		seededForRef.current = sourceClipId;
		// The draft's sources still need their buffers so the timeline plays.
		void editor.preloadSources(
			props.guildId,
			draft.segments.map((segment) => segment.sourceId),
		);
	}, [editor, props.guildId, sourceClipId]);

	// Persist the draft after every committed change; the debounce folds drag
	// previews into one write. Empty edits are not stored, so a fresh session
	// never creates a draft that shadows a later seed.
	useEffect(() => {
		if (editor.edit.segments.length === 0) return;
		const timeout = window.setTimeout(
			() => saveDraft(props.guildId, sourceClipId, editor.edit),
			400,
		);
		return () => window.clearTimeout(timeout);
	}, [editor.edit, props.guildId, sourceClipId]);

	// Loads the original version of the source clip - its stored composition
	// or a single plain segment - into the editor. Returns whether the seed
	// could be built (the clip data may still be loading).
	const seedFromSource = useCallback((): boolean => {
		if (!sourceClipId) return false;
		const source = clips?.find((c) => c.clip_id === sourceClipId);
		const edit = source?.composition
			? deserializeEdit(source.composition)
			: null;
		if (edit && edit.segments.length > 0) {
			// Composed clip: restore the whole edit, then load every source
			// buffer so the timeline plays as it did when it was exported.
			// Reset (not apply) makes the restored edit the undo baseline.
			editor.reset(edit);
			editor.select(edit.segments[0]?.id ?? null);
			void editor.preloadSources(
				props.guildId,
				edit.segments.map((segment) => segment.sourceId),
			);
			return true;
		}
		if (!sourceBuffer) return false;
		// The composition marker only exists in the clips list; without it a
		// composed source would be seeded as a single plain segment, so wait
		// for the list before assuming the source is a plain clip.
		if (clips === undefined && !clipsError) return false;
		editor.registerBuffer(sourceClipId, sourceBuffer);
		const lengthSec = source?.length ?? sourceBuffer.duration;
		const segment = makeSegment("clip", sourceClipId, 0, lengthSec, 0, 0);
		editor.reset(addSegment(editor.edit, segment));
		editor.select(segment.id);
		return true;
	}, [clips, clipsError, editor, props.guildId, sourceBuffer, sourceClipId]);

	useEffect(() => {
		if (seededForRef.current === sourceClipId || !sourceClipId) return;
		if (seedFromSource()) seededForRef.current = sourceClipId;
	}, [seedFromSource, sourceClipId]);

	// Rebuilds the clip's original version on demand; apply (not reset) keeps
	// the restore undoable, and the draft save picks up the change.
	const restoreOriginal = useCallback(() => {
		if (!sourceClipId) return;
		const source = clips?.find((c) => c.clip_id === sourceClipId);
		const edit = source?.composition
			? deserializeEdit(source.composition)
			: null;
		if (edit && edit.segments.length > 0) {
			editor.apply(() => edit);
			editor.select(edit.segments[0]?.id ?? null);
			return;
		}
		if (!sourceBuffer) return;
		if (clips === undefined && !clipsError) return;
		const lengthSec = source?.length ?? sourceBuffer.duration;
		const segment = makeSegment("clip", sourceClipId, 0, lengthSec, 0, 0);
		editor.apply(() => addSegment(emptyEdit(), segment));
		editor.select(segment.id);
	}, [clips, clipsError, editor, sourceBuffer, sourceClipId]);

	const handleDropClip = useCallback(
		(clipId: string, lengthSec: number, track: number, startSec: number) => {
			editor.loadClip(props.guildId, clipId, lengthSec, track, startSec);
		},
		[editor, props.guildId],
	);

	const handleAddFromBin = useCallback(
		(clip: ClipData) => {
			editor.loadClip(
				props.guildId,
				clip.clip_id,
				clip.length ?? 1,
				editor.activeTrack,
			);
		},
		[editor, props.guildId],
	);

	useKeyboardShortcuts(editor, () => setEffectSettingsJsonOpen(true));

	const pureClips = (clips ?? []).filter((clip) => !isComposedClip(clip));

	// Overwriting replaces the composed clip this editor was opened from; it
	// only makes sense when the source is a combined clip, not a raw one.
	const sourceClip = clips?.find((clip) => clip.clip_id === sourceClipId);
	const canOverwrite =
		sourceClipId !== null &&
		sourceClip !== undefined &&
		isComposedClip(sourceClip);

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
				canRestore={sourceClipId !== null}
				onRestore={restoreOriginal}
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
						sourceError={sourceError}
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
				<Inspector
					editor={editor}
					clipName={clipName}
					limits={effectLimits}
					onOpenLimits={() => setEffectLimitsOpen(true)}
				/>
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
				overwriteAvailable={canOverwrite}
				overwrite={overwrite}
				setOverwrite={setOverwrite}
				onStart={() => void handleCompose()}
				onClose={closeCompose}
			/>
			<EffectSettingsJsonDialog
				open={effectSettingsJsonOpen}
				onClose={() => setEffectSettingsJsonOpen(false)}
				editor={editor}
			/>
			<EffectLimitsDialog
				open={effectLimitsOpen}
				onClose={() => setEffectLimitsOpen(false)}
				limits={effectLimits}
				onChange={updateEffectLimits}
			/>
			{unsavedDialog}
			<Snackbar
				open={editor.mergeWarning !== null}
				anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
				autoHideDuration={5000}
				onClose={editor.dismissMergeWarning}
			>
				<Alert
					severity="warning"
					variant="filled"
					onClose={editor.dismissMergeWarning}
					sx={{ alignItems: "center" }}
				>
					{editor.mergeWarning}
				</Alert>
			</Snackbar>
		</Box>
	);
}

function ToolbarRow(props: {
	editor: ReturnType<typeof useClipEditor>;
	onExport: () => void;
	canExport: boolean;
	canRestore: boolean;
	onRestore: () => void;
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
			{props.canRestore && (
				<Tooltip title="Restore the clip to its original version (can be undone)">
					<span>
						<IconButton size="small" onClick={props.onRestore}>
							<RestoreIcon fontSize="small" />
						</IconButton>
					</span>
				</Tooltip>
			)}
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
			<Tooltip title="Export the composition as a new clip or overwrite the combined clip">
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
	overwriteAvailable: boolean;
	overwrite: boolean;
	setOverwrite: (overwrite: boolean) => void;
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
							{props.overwrite ? "Overwrite" : "Render"}
						</Button>
					)}
				</>
			}
		>
			{/* Fixed footprint: the overwrite/new toggle and helper text change
			    the content height, so pin the box size to keep the dialog from
			    resizing while the choice changes. */}
			<Box
				sx={{
					width: 440,
					minHeight: 230,
					display: "flex",
					flexDirection: "column",
				}}
			>
				{props.overwriteAvailable && (
					<FormControl component="fieldset" disabled={busy} sx={{ mb: 2 }}>
						<RadioGroup
							value={props.overwrite ? "overwrite" : "new"}
							onChange={(event) =>
								props.setOverwrite(event.currentTarget.value === "overwrite")
							}
						>
							<FormControlLabel
								value="new"
								control={<Radio size="small" />}
								label="Save as new clip"
							/>
							<FormControlLabel
								value="overwrite"
								control={<Radio size="small" />}
								label="Overwrite this combined clip"
							/>
						</RadioGroup>
					</FormControl>
				)}
				<TextField
					size="small"
					fullWidth
					label="Clip name"
					value={props.name}
					disabled={busy}
					onChange={(event) => props.setName(event.currentTarget.value)}
					helperText={
						props.overwrite
							? "Leave empty to keep the current clip's name."
							: undefined
					}
					sx={{ mb: 2 }}
				/>
				{props.done ? (
					<Typography variant="body2">
						{props.overwrite
							? "Updated — the combined clip now reflects this version."
							: "Exported — the new clip is now in the bin."}
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
						{props.overwrite
							? `Renders ${props.segmentCount} segment${props.segmentCount === 1 ? "" : "s"} and replaces the combined clip with this version.`
							: `Renders ${props.segmentCount} segment${props.segmentCount === 1 ? "" : "s"} into a single new clip.`}
					</Typography>
				)}
			</Box>
		</BaseDialog>
	);
}

function Monitor(props: {
	editor: ReturnType<typeof useClipEditor>;
	duration: number;
	sourceStatus: "idle" | "loading" | "ready" | "error";
	sourceError: string | null;
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
					{props.sourceError ?? "Source clip could not be loaded."}
				</Alert>
			)}
		</Box>
	);
}

function useKeyboardShortcuts(
	editor: ReturnType<typeof useClipEditor>,
	openEffectSettingsJson: () => void,
) {
	const editorRef = useRef(editor);
	const openEffectSettingsJsonRef = useRef(openEffectSettingsJson);
	useEffect(() => {
		editorRef.current = editor;
		openEffectSettingsJsonRef.current = openEffectSettingsJson;
	}, [editor, openEffectSettingsJson]);

	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			const current = editorRef.current;
			if (isEffectSettingsJsonShortcut(event)) {
				event.preventDefault();
				openEffectSettingsJsonRef.current();
				return;
			}
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
			if (modifier && event.key.toLowerCase() === "c") {
				if (current.selectedSegment) {
					event.preventDefault();
					current.copy();
				}
				return;
			}
			if (modifier && event.key.toLowerCase() === "x") {
				if (current.selectedSegment) {
					event.preventDefault();
					current.cut();
				}
				return;
			}
			if (modifier && event.key.toLowerCase() === "v") {
				event.preventDefault();
				current.paste();
				return;
			}
			if (modifier && event.key.toLowerCase() === "a") {
				event.preventDefault();
				current.selectMany(current.edit.segments.map((segment) => segment.id));
				return;
			}
			if (modifier) return;
			if (event.key === "Escape") {
				if (current.selectedSegments.length > 0) {
					event.preventDefault();
					current.select(null);
				}
				return;
			}
			if (event.key === " " || event.code === "Space") {
				if (event.repeat) return;
				event.preventDefault();
				current.togglePlay();
				return;
			}
			const segment = current.selectedSegment;
			if (event.key === "Delete" || event.key === "Backspace") {
				if (segment) {
					if (
						isInspectorFeatureDisabled(
							"delete",
							current.selectedSegments.length,
						)
					)
						return;
					event.preventDefault();
					current.removeSelected();
				}
				return;
			}
			if (segment) {
				if (event.key === "s" || event.key === "S") {
					if (
						isInspectorFeatureDisabled("split", current.selectedSegments.length)
					)
						return;
					event.preventDefault();
					current.splitSelectedAtPlayhead();
					return;
				}
				if (event.key === "r" || event.key === "R") {
					if (
						isInspectorFeatureDisabled(
							"reverse",
							current.selectedSegments.length,
						)
					)
						return;
					event.preventDefault();
					current.toggleReverse();
					return;
				}
				if (event.key === "m" || event.key === "M") {
					event.preventDefault();
					current.mergeSelected();
					return;
				}
			}
			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			const distance = event.shiftKey ? 1 : 0.1;
			event.preventDefault();
			current.setPosition(
				Math.max(
					0,
					current.positionSec +
						(event.key === "ArrowRight" ? distance : -distance),
				),
			);
			return;
		};
		window.addEventListener("keydown", handleShortcut);
		return () => window.removeEventListener("keydown", handleShortcut);
	}, []);
}
