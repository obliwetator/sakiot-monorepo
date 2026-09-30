import { FolderOpen as FolderOpenIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { type ClipData, useGetClipsQuery } from "../../app/apiSlice";
import { useAsRole } from "../../app/useAsRole";
import { Button, Drawer, Notice, useMediaQuery } from "../../shared/ui";
import { isComposedClip } from "../clips/composedClip";
import { ClipBin } from "./ClipBin";
import { ClipEditorMonitor, ClipEditorToolbar } from "./ClipEditorChrome";
import { ClipExportDialog } from "./ClipExportDialog";
import { deserializeEdit, serializeEdit } from "./composePayload";
import { DraftNotices } from "./DraftStatus";
import { DraftPersistence } from "./draftPersistence";
import { EditorOptionsDialog } from "./EditorOptionsDialog";
import { EffectLimitsDialog } from "./EffectLimitsDialog";
import { EffectSettingsJsonDialog } from "./EffectSettingsJsonDialog";
import {
	type EditorOptions,
	loadEditorOptions,
	saveEditorOptions,
} from "./editorOptions";
import {
	type EffectLimits,
	loadEffectLimits,
	saveEffectLimits,
} from "./effectLimits";
import { Inspector } from "./Inspector";
import {
	addSegment,
	type ClipEdit,
	emptyEdit,
	makeSegment,
	segmentDuration,
} from "./model";
import {
	type MobileBinDragPreview,
	type MobileBinDropRequest,
	Timeline,
} from "./Timeline";
import { useUnsavedChangesGuard } from "./unsavedChangesGuard";
import { useClipBuffer } from "./useClipBuffer";
import { useClipEditor } from "./useClipEditor";
import { useClipEditorKeyboardShortcuts } from "./useClipEditorKeyboardShortcuts";
import { useCompositionExport } from "./useCompositionExport";
import { useDraftPersistence } from "./useDraftPersistence";

/**
 * One editing session: one user, guild and source clip. The page remounts
 * the editor when any of them changes, so a session never outlives its draft.
 */
export function ClipEditor(props: {
	guildId: string;
	userId: string;
	sourceClipId: string | null;
}) {
	const isDesktop = useMediaQuery("(min-width: 900px)");
	const isTouchInput = useMediaQuery("(hover: none), (pointer: coarse)");
	const [clipBinOpen, setClipBinOpen] = useState(false);
	const binDragStateRef = useRef<"idle" | "dragging" | "accepted">("idle");
	// The source must survive the gesture, but the closed drawer must not
	// keep the timeline inert or retain the modal focus trap.
	const [binGestureActive, setBinGestureActive] = useState(false);
	const mobileDropIdRef = useRef(0);
	const [mobileBinDrop, setMobileBinDrop] =
		useState<MobileBinDropRequest | null>(null);
	const [mobileDragGhost, setMobileDragGhost] = useState<
		| (MobileBinDragPreview & {
				name: string;
		  })
		| null
	>(null);
	const { asRoleArg } = useAsRole();
	const { currentData: clips, isError: clipsError } = useGetClipsQuery(
		{ guild_id: props.guildId, ...asRoleArg },
		{ skip: !props.guildId },
	);
	const [optionsOpen, setOptionsOpen] = useState(false);
	const [options, setOptions] = useState<EditorOptions>(() =>
		loadEditorOptions(),
	);
	const { sourceClipId } = props;
	// Restores synchronously, so the editor opens on the stored draft and
	// nothing can be saved before restoration has happened.
	const [drafts] = useState(
		() =>
			new DraftPersistence({
				userId: props.userId,
				guildId: props.guildId,
				sourceClipId,
			}),
	);
	const editor = useClipEditor({
		copyAllSelected: options.copyAllSelected,
		initialEdit: drafts.initialEdit,
	});
	// What the editor opened with. Without a stored draft a source clip still
	// has to load before the editor knows its original version; until then
	// this is null and nothing is saved.
	const [baseline, setBaseline] = useState<ClipEdit | null>(() =>
		drafts.restored || !sourceClipId ? drafts.initialEdit : null,
	);
	const replaceEdit = useCallback(
		(next: ClipEdit) => {
			editor.apply(() => next);
			editor.select(null);
			void editor.preloadSources(
				props.guildId,
				next.segments.map((segment) => segment.sourceId),
			);
		},
		[editor, props.guildId],
	);
	const draft = useDraftPersistence({
		session: drafts,
		edit: editor.edit,
		committed: editor.committed,
		baseline,
		replaceEdit,
	});
	const { dialog: unsavedDialog } = useUnsavedChangesGuard(draft);

	useEffect(() => {
		if (isDesktop) {
			setClipBinOpen(false);
			setMobileBinDrop(null);
			setBinGestureActive(false);
			binDragStateRef.current = "idle";
			setMobileDragGhost(null);
		}
	}, [isDesktop]);

	const [composeOpen, setComposeOpen] = useState(false);
	const [effectSettingsJsonOpen, setEffectSettingsJsonOpen] = useState(false);
	const [effectLimitsOpen, setEffectLimitsOpen] = useState(false);
	const [effectLimits, setEffectLimits] = useState<EffectLimits>(() =>
		loadEffectLimits(),
	);
	const [composeName, setComposeName] = useState("");
	const [overwrite, setOverwrite] = useState(false);
	const exportJob = useCompositionExport(props.guildId);
	const handleCompose = () => {
		if (editor.edit.segments.length === 0) return;
		editor.flush();
		exportJob.begin(
			serializeEdit(
				editor.edit,
				composeName.trim() || undefined,
				overwrite ? (sourceClipId ?? undefined) : undefined,
				effectLimits,
			),
		);
	};

	const updateEffectLimits = useCallback((limits: EffectLimits) => {
		setEffectLimits(limits);
		saveEffectLimits(limits);
	}, []);

	const updateOptions = useCallback((next: EditorOptions) => {
		setOptions(next);
		saveEditorOptions(next);
	}, []);

	const closeCompose = () => {
		setComposeOpen(false);
		exportJob.resetMessage();
	};

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

	// The original version of the source clip: its stored composition, or one
	// plain segment. "loading" until the clip list and audio have arrived;
	// "unavailable" when the clip cannot be opened.
	const originalEdit = useCallback((): ClipEdit | "loading" | "unavailable" => {
		if (!sourceClipId) return "unavailable";
		const source = clips?.find((c) => c.clip_id === sourceClipId);
		const composed = source?.composition
			? deserializeEdit(source.composition)
			: null;
		if (composed && composed.segments.length > 0) return composed;
		// The composition marker only exists in the clips list; without it a
		// composed source would be seeded as a single plain segment, so wait
		// for the list before assuming the source is a plain clip.
		if (clips === undefined && !clipsError) return "loading";
		if (!sourceBuffer) {
			return sourceStatus === "error" ? "unavailable" : "loading";
		}
		editor.registerBuffer(sourceClipId, sourceBuffer);
		const lengthSec = source?.length ?? sourceBuffer.duration;
		const segment = makeSegment("clip", sourceClipId, 0, lengthSec, 0, 0);
		return addSegment(emptyEdit(), segment);
	}, [clips, clipsError, editor, sourceBuffer, sourceClipId, sourceStatus]);

	// Without a stored draft, open the source clip's original version. Reset
	// (not apply) makes it the undo baseline. If the user edits first, or the
	// clip cannot be opened, keep the current edit and start saving it.
	useEffect(() => {
		if (baseline !== null) return;
		if (editor.edit !== drafts.initialEdit) {
			setBaseline(drafts.initialEdit);
			return;
		}
		const original = originalEdit();
		if (original === "loading") return;
		if (original === "unavailable") {
			setBaseline(drafts.initialEdit);
			return;
		}
		editor.reset(original);
		setBaseline(original);
	}, [baseline, drafts, editor, originalEdit]);

	// Once the editor has opened on a draft or the original clip, select its
	// first segment and load every source so the timeline plays.
	const openedRef = useRef<ClipEdit | null>(null);
	useEffect(() => {
		if (baseline === null || openedRef.current === baseline) return;
		openedRef.current = baseline;
		if (editor.edit !== baseline) return;
		editor.select(baseline.segments[0]?.id ?? null);
		void editor.preloadSources(
			props.guildId,
			baseline.segments.map((segment) => segment.sourceId),
		);
	}, [baseline, editor, props.guildId]);

	// Rebuilds the clip's original version on demand; apply (not reset) keeps
	// the restore undoable, and the draft save picks up the change.
	const restoreOriginal = useCallback(() => {
		const original = originalEdit();
		if (typeof original === "string") return;
		replaceEdit(original);
	}, [originalEdit, replaceEdit]);

	const completeMobileBinLoad = useCallback(
		(success: boolean) => {
			binDragStateRef.current = "idle";
			if (!isDesktop && !success) setClipBinOpen(true);
		},
		[isDesktop],
	);

	const handleDropClip = useCallback(
		(clipId: string, lengthSec: number, track: number, startSec: number) => {
			if (!isDesktop) binDragStateRef.current = "accepted";
			void editor
				.loadClip(props.guildId, clipId, lengthSec, track, startSec)
				.then(completeMobileBinLoad);
		},
		[completeMobileBinLoad, editor, isDesktop, props.guildId],
	);

	const handleAddFromBin = useCallback(
		(clip: ClipData) => {
			if (!isDesktop) {
				binDragStateRef.current = "accepted";
				setClipBinOpen(false);
			}
			void editor
				.loadClip(
					props.guildId,
					clip.clip_id,
					clip.length ?? 1,
					editor.activeTrack,
				)
				.then(completeMobileBinLoad);
		},
		[completeMobileBinLoad, editor, isDesktop, props.guildId],
	);

	const handleBinDragStart = useCallback(() => {
		if (isDesktop) return;
		binDragStateRef.current = "dragging";
		setBinGestureActive(true);
		setClipBinOpen(false);
	}, [isDesktop]);

	const handleBinDragEnd = useCallback(() => {
		setBinGestureActive(false);
		if (binDragStateRef.current !== "dragging") return;
		binDragStateRef.current = "idle";
		if (!isDesktop) setClipBinOpen(true);
	}, [isDesktop]);

	const handleTouchDragStart = useCallback(
		(clip: ClipData, clientX: number, clientY: number) => {
			binDragStateRef.current = "dragging";
			setBinGestureActive(true);
			if (!isDesktop) setClipBinOpen(false);
			setMobileDragGhost({
				name: clip.name || "Unnamed clip",
				clipId: clip.clip_id,
				lengthSec: clip.length ?? 0,
				clientX,
				clientY,
			});
		},
		[isDesktop],
	);

	const handleTouchDragMove = useCallback(
		(_clip: ClipData, clientX: number, clientY: number) => {
			setMobileDragGhost((current) =>
				current ? { ...current, clientX, clientY } : current,
			);
		},
		[],
	);

	const handleTouchDrop = useCallback(
		(clip: ClipData, clientX: number, clientY: number) => {
			setBinGestureActive(false);
			setClipBinOpen(false);
			mobileDropIdRef.current += 1;
			setMobileBinDrop({
				id: mobileDropIdRef.current,
				clipId: clip.clip_id,
				lengthSec: clip.length ?? 0,
				clientX,
				clientY,
			});
			setMobileDragGhost(null);
		},
		[],
	);

	const handleTouchDragCancel = useCallback(() => {
		setBinGestureActive(false);
		setMobileDragGhost(null);
		handleBinDragEnd();
	}, [handleBinDragEnd]);

	const handleMobileBinDropHandled = useCallback(
		(id: number, accepted: boolean) => {
			setMobileBinDrop((current) => (current?.id === id ? null : current));
			if (!accepted) handleBinDragEnd();
		},
		[handleBinDragEnd],
	);

	useClipEditorKeyboardShortcuts(
		editor,
		() => setEffectSettingsJsonOpen(true),
		() => setOptionsOpen(true),
	);

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
		<div className="h-full min-h-0 flex flex-col">
			<ClipEditorToolbar
				editor={editor}
				draftStatus={draft.status}
				onExport={() => setComposeOpen(true)}
				canExport={editor.edit.segments.length > 0}
				canRestore={sourceClipId !== null}
				onRestore={restoreOriginal}
				onOpenOptions={() => setOptionsOpen(true)}
			/>
			<DraftNotices draft={draft} />
			<div className="flex-1 min-h-0 min-w-0 flex flex-col min-[900px]:flex-row overflow-hidden">
				{isDesktop ? (
					<ClipBin
						clips={pureClips}
						loadingClips={editor.loadingClips}
						onAdd={handleAddFromBin}
						onTouchDragStart={handleTouchDragStart}
						onTouchDragMove={handleTouchDragMove}
						onTouchDrop={handleTouchDrop}
						onTouchDragCancel={handleTouchDragCancel}
						tapToAdd={isTouchInput}
						disableNativeDrag={isTouchInput}
					/>
				) : (
					<>
						<div className="flex-none p-2 border-b border-ui-border">
							<Button
								className="w-full"
								variant="outline"
								onPress={() => setClipBinOpen(true)}
							>
								<FolderOpenIcon />
								Browse files
							</Button>
						</div>
						<Drawer
							isOpen={clipBinOpen}
							isExiting={binGestureActive && !clipBinOpen}
							aria-label="Browse files"
							side={"left"}
							onOpenChange={(isOpen) => {
								if (binDragStateRef.current === "idle") setClipBinOpen(isOpen);
							}}
						>
							<ClipBin
								clips={pureClips}
								loadingClips={editor.loadingClips}
								onAdd={handleAddFromBin}
								onDragStart={handleBinDragStart}
								onDragEnd={handleBinDragEnd}
								onTouchDragStart={handleTouchDragStart}
								onTouchDragMove={handleTouchDragMove}
								onTouchDrop={handleTouchDrop}
								onTouchDragCancel={handleTouchDragCancel}
								tapToAdd
								disableNativeDrag={isTouchInput}
							/>
						</Drawer>
					</>
				)}
				<div className="flex-1 min-w-0 min-h-0 w-full flex flex-col">
					<ClipEditorMonitor
						editor={editor}
						duration={duration}
						sourceStatus={sourceStatus}
						sourceError={sourceError}
					/>
					<div className="flex-1 min-h-0">
						<Timeline
							guildId={props.guildId}
							editor={editor}
							clipName={(segment) => clipName(segment.sourceId)}
							onDropClip={handleDropClip}
							mobileBinDragPreview={mobileDragGhost}
							mobileBinDrop={mobileBinDrop}
							onMobileBinDropHandled={handleMobileBinDropHandled}
							multiTrackMarquee={options.marqueeMultiTrack}
							audacityStyleInteraction={options.audacityStyleInteraction}
						/>
					</div>
				</div>
				<Inspector
					editor={editor}
					clipName={clipName}
					limits={effectLimits}
					onOpenLimits={() => setEffectLimitsOpen(true)}
				/>
			</div>
			{mobileDragGhost && (
				<div
					aria-hidden="true"
					className="fixed max-w-55 px-2.5 py-1.5 border border-dashed border-accent rounded-[1px] bg-surface [box-shadow:4px] text-xs font-semibold whitespace-nowrap overflow-hidden text-ellipsis pointer-events-none"
					style={{
						zIndex: 70,
						...{
							left: mobileDragGhost.clientX + 12,
							top: mobileDragGhost.clientY + 12,
						},
					}}
				>
					{mobileDragGhost.name}
				</div>
			)}
			<ClipExportDialog
				open={composeOpen}
				name={composeName}
				setName={setComposeName}
				error={exportJob.error}
				isStarting={exportJob.starting}
				isRendering={exportJob.rendering}
				progress={exportJob.progress}
				stage={exportJob.stage}
				retryReason={exportJob.retryReason}
				done={exportJob.done}
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
				limits={effectLimits}
			/>
			<EffectLimitsDialog
				open={effectLimitsOpen}
				onClose={() => setEffectLimitsOpen(false)}
				limits={effectLimits}
				onChange={updateEffectLimits}
			/>
			<EditorOptionsDialog
				open={optionsOpen}
				onClose={() => setOptionsOpen(false)}
				options={options}
				onChange={updateOptions}
				onOpenDraftFile={draft.openFile}
			/>
			{unsavedDialog}
			{editor.mergeWarning !== null && (
				<div className="fixed bottom-4 left-1/2 z-[60] -translate-x-1/2">
					<Notice className="items-center" tone={"warning"} announce="status">
						{editor.mergeWarning}
						<Button
							variant="ghost"
							size="sm"
							onPress={editor.dismissMergeWarning}
						>
							Dismiss
						</Button>
					</Notice>
				</div>
			)}
			{editor.effectsUnavailable && (
				<div className="fixed bottom-4 left-1/2 z-[60] -translate-x-1/2">
					<Notice className="items-center" tone={"warning"} announce="status">
						Effects unavailable: this preview is playing without pitch, speed,
						and reverse. Exports still apply them.
					</Notice>
				</div>
			)}
		</div>
	);
}
