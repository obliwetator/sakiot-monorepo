import {
	Copy as ContentCopyIcon,
	Maximize as FitScreenIcon,
	Pause as PauseIcon,
	Play as PlayArrowIcon,
	Redo2 as RedoIcon,
	Repeat as RepeatIcon,
	History as RestoreIcon,
	Settings as SettingsIcon,
	Undo2 as UndoIcon,
} from "lucide-react";
import {
	Alert,
	Box,
	Button,
	Chip,
	IconButton,
	Slider,
	Tooltip,
	Typography,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { addTrack } from "./model";
import type { UseClipEditorReturn } from "./useClipEditor";

export function ClipEditorToolbar(props: {
	editor: UseClipEditorReturn;
	onExport: () => void;
	canExport: boolean;
	canRestore: boolean;
	onRestore: () => void;
	onOpenOptions: () => void;
}) {
	const { editor } = props;
	return (
		<Box
			sx={{
				display: "flex",
				alignItems: "center",
				gap: { xs: 0.5, sm: 1 },
				px: { xs: 1, sm: 2 },
				py: 1,
				borderBottom: 1,
				borderColor: "divider",
				overflowX: "auto",
				flex: "0 0 auto",
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
						<UndoIcon size={16} />
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
						<RedoIcon size={16} />
					</IconButton>
				</span>
			</Tooltip>
			{props.canRestore && (
				<Tooltip title="Restore the clip to its original version (can be undone)">
					<span>
						<IconButton size="small" onClick={props.onRestore}>
							<RestoreIcon size={16} />
						</IconButton>
					</span>
				</Tooltip>
			)}
			<Tooltip title="Add track">
				<Button
					size="small"
					variant="outlined"
					onClick={() => editor.apply(addTrack)}
				>
					+ Track
				</Button>
			</Tooltip>
			<Tooltip title="Fit edit in view">
				<IconButton size="small" onClick={editor.fitView}>
					<FitScreenIcon size={16} />
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
			<Tooltip title="Editor options (Ctrl+,)">
				<IconButton size="small" onClick={props.onOpenOptions}>
					<SettingsIcon size={16} />
				</IconButton>
			</Tooltip>
		</Box>
	);
}

export function ClipEditorMonitor(props: {
	editor: UseClipEditorReturn;
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
				gap: { xs: 1, sm: 2 },
				px: { xs: 1, sm: 2 },
				py: { xs: 0.5, sm: 1 },
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
					<RepeatIcon size={16} />
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
					sx={{ transition: "none" }}
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
