import type { SelectionEdge } from "./clipSelection";
import type { SessionSelection } from "./logicalSessionSelection";

export interface ClipRangeEditorProps {
	sessionId: string;
	durationMs: number;
	selection: SessionSelection;
	/** One-time centre for an editor opened from a stamp. */
	initialFocusMs?: number;
	onSelectionChange: (next: SessionSelection) => void;
	positionMs: number;
	onSeek: (positionMs: number) => void;
	/** Shows a playhead position while scrubbing without reloading audio. */
	onSeekPreview?: (positionMs: number | null) => void;
	onSetEdgeFromPlayhead: (edge: SelectionEdge) => void;
	onSetNearestEdgeFromPlayhead: () => void;
	edgeHint?: string | null;
	onReset: () => void;
	onPreview: () => void;
	previewing: boolean;
	loop: boolean;
	onLoopChange: (loop: boolean) => void;
	/** Draw peaks from compressed silence-free session timeline. */
	silenceFree?: boolean;
}
