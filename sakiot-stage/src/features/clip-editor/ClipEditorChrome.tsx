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
	Notice,
	Slider,
	TooltipIconButton,
	WithTooltip,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { DraftStatusIndicator } from "./DraftStatus";
import type { DraftStatus } from "./draftPersistence";
import { addTrack } from "./model";
import {
	BROWSER_PREVIEW_LIMIT_MESSAGE,
	browserPreviewLimited,
} from "./pcmBudget";
import type { UseClipEditorReturn } from "./useClipEditor";

export function ClipEditorToolbar(props: {
	editor: UseClipEditorReturn;
	onExport: () => void;
	canExport: boolean;
	canRestore: boolean;
	onRestore: () => void;
	onOpenOptions: () => void;
	draftStatus: DraftStatus;
}) {
	const { editor } = props;
	return (
		<div className="flex items-center gap-1 min-[600px]:gap-2 px-2 min-[600px]:px-4 py-2 border-b border-ui-border overflow-x-auto flex-none">
			<h6 className="font-medium tracking-[0.001em] text-xl truncate flex-1 min-w-0">
				Clip Editor
			</h6>
			<DraftStatusIndicator status={props.draftStatus} />
			<TooltipIconButton
				label="Undo (Ctrl+Z)"
				icon={<UndoIcon size={16} />}
				isDisabled={!editor.canUndo}
				onPress={editor.undo}
			/>
			<TooltipIconButton
				label="Redo (Ctrl+Shift+Z)"
				icon={<RedoIcon size={16} />}
				isDisabled={!editor.canRedo}
				onPress={editor.redo}
			/>
			{props.canRestore && (
				<TooltipIconButton
					label="Restore original clip"
					tip="Restore the clip to its original version (can be undone)"
					icon={<RestoreIcon size={16} />}
					onPress={props.onRestore}
				/>
			)}
			<WithTooltip tip="Add track">
				<Button
					variant="outline"
					size="sm"
					onPress={() => editor.apply(addTrack)}
				>
					+ Track
				</Button>
			</WithTooltip>
			<TooltipIconButton
				label="Fit edit in view"
				icon={<FitScreenIcon size={16} />}
				onPress={editor.fitView}
			/>
			<WithTooltip tip="Export the composition as a new clip or overwrite the combined clip">
				<Button
					variant="primary"
					size="sm"
					isDisabled={!props.canExport}
					onPress={props.onExport}
				>
					<ContentCopyIcon />
					Export
				</Button>
			</WithTooltip>
			<TooltipIconButton
				label="Editor options (Ctrl+,)"
				icon={<SettingsIcon size={16} />}
				onPress={props.onOpenOptions}
			/>
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
				isDisabled={props.duration <= 0 || browserPreviewLimited(editor.edit)}
				onPress={editor.togglePlay}
			>
				{editor.playing ? <PauseIcon /> : <PlayArrowIcon />}
				{editor.playing ? "Pause" : "Play"}
			</Button>
			<TooltipIconButton
				label="Loop the edit while playing"
				aria-pressed={editor.loop}
				icon={<RepeatIcon size={16} />}
				onPress={() => editor.setLooping(!editor.loop)}
			/>
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
			{browserPreviewLimited(editor.edit) && (
				<Notice className="py-0" tone="warning" announce="status">
					{BROWSER_PREVIEW_LIMIT_MESSAGE}
				</Notice>
			)}
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
