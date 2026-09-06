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
	Badge,
	Button,
	IconButton,
	Notice,
	Slider,
	Tooltip,
	TooltipTrigger,
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
		<div className="flex items-center gap-1 min-[600px]:gap-2 px-2 min-[600px]:px-4 py-2 border-b border-ui-border overflow-x-auto flex-none">
			<h6 className="font-medium tracking-[0.001em] text-xl truncate flex-1 min-w-0">
				Clip Editor
			</h6>
			<TooltipTrigger delay={400}>
				<IconButton
					aria-label={"Undo (Ctrl+Z)"}
					size="sm"
					isDisabled={!editor.canUndo}
					onPress={editor.undo}
				>
					<UndoIcon size={16} />
				</IconButton>
				<Tooltip>{"Undo (Ctrl+Z)"}</Tooltip>
			</TooltipTrigger>
			<TooltipTrigger delay={400}>
				<IconButton
					aria-label={"Redo (Ctrl+Shift+Z)"}
					size="sm"
					isDisabled={!editor.canRedo}
					onPress={editor.redo}
				>
					<RedoIcon size={16} />
				</IconButton>
				<Tooltip>{"Redo (Ctrl+Shift+Z)"}</Tooltip>
			</TooltipTrigger>
			{props.canRestore && (
				<TooltipTrigger delay={400}>
					<IconButton
						aria-label="Restore original clip"
						size="sm"
						onPress={props.onRestore}
					>
						<RestoreIcon size={16} />
					</IconButton>
					<Tooltip>
						{"Restore the clip to its original version (can be undone)"}
					</Tooltip>
				</TooltipTrigger>
			)}
			<TooltipTrigger delay={400}>
				<Button
					variant="outline"
					size="sm"
					onPress={() => editor.apply(addTrack)}
				>
					+ Track
				</Button>
				<Tooltip>{"Add track"}</Tooltip>
			</TooltipTrigger>
			<TooltipTrigger delay={400}>
				<IconButton
					aria-label={"Fit edit in view"}
					size="sm"
					onPress={editor.fitView}
				>
					<FitScreenIcon size={16} />
				</IconButton>
				<Tooltip>{"Fit edit in view"}</Tooltip>
			</TooltipTrigger>
			<TooltipTrigger delay={400}>
				<Button
					variant="primary"
					size="sm"
					isDisabled={!props.canExport}
					onPress={props.onExport}
				>
					<ContentCopyIcon />
					Export
				</Button>
				<Tooltip>
					{
						"Export the composition as a new clip or overwrite the combined clip"
					}
				</Tooltip>
			</TooltipTrigger>
			<TooltipTrigger delay={400}>
				<IconButton
					aria-label={"Editor options (Ctrl+,)"}
					size="sm"
					onPress={props.onOpenOptions}
				>
					<SettingsIcon size={16} />
				</IconButton>
				<Tooltip>{"Editor options (Ctrl+,)"}</Tooltip>
			</TooltipTrigger>
		</div>
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
		<div className="flex items-center gap-2 min-[600px]:gap-4 px-2 min-[600px]:px-4 py-1 min-[600px]:py-2 border-b border-ui-border flex-wrap">
			<Button
				variant="primary"
				isDisabled={props.duration <= 0}
				onPress={editor.togglePlay}
			>
				{editor.playing ? <PauseIcon /> : <PlayArrowIcon />}
				{editor.playing ? "Pause" : "Play"}
			</Button>
			<TooltipTrigger delay={400}>
				<IconButton
					aria-label={"Loop the edit while playing"}
					aria-pressed={editor.loop}
					size="sm"
					onPress={() => editor.setLooping(!editor.loop)}
				>
					<RepeatIcon size={16} />
				</IconButton>
				<Tooltip>{"Loop the edit while playing"}</Tooltip>
			</TooltipTrigger>
			<p className="text-sm tabular-nums">
				{formatDuration(editor.positionSec)} / {formatDuration(props.duration)}
			</p>
			<div className="min-w-40 w-50">
				<span className="text-muted text-xs leading-5">
					Master volume {editor.masterVolumeDb.toFixed(1)} dB
				</span>
				<Slider
					step={0.5}
					value={editor.masterVolumeDb}
					aria-label="Master volume"
					className="transition-none"
					minValue={-40}
					maxValue={12}
					onChange={(value) => editor.setMasterVolume(Number(value))}
				/>
			</div>
			<Badge
				size={"sm"}
			>{`${editor.edit.segments.length} segment${editor.edit.segments.length === 1 ? "" : "s"}`}</Badge>
			{props.sourceStatus === "loading" && (
				<Badge tone={"warning"} size={"sm"}>
					Loading source…
				</Badge>
			)}
			{props.sourceStatus === "error" && (
				<Notice className="py-0" tone={"error"} announce="alert">
					{props.sourceError ?? "Source clip could not be loaded."}
				</Notice>
			)}
		</div>
	);
}
