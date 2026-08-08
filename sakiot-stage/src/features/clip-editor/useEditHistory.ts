import { useCallback, useState } from "react";
import {
	createHistoryState,
	type HistoryState,
	historyApply,
	historyFlush,
	historyPreview,
	historyRedo,
	historyUndo,
} from "./history";
import type { ClipEdit } from "./model";

/**
 * Undo/redo with gesture batching, backed by the pure `history` transitions.
 * `preview` mutates a pending draft, `flush` folds every preview since the
 * last flush into one history step. Timeline drags preview on every
 * pointermove and flush on pointerup, so a whole drag is one undo step.
 */
export function useEditHistory(initial: ClipEdit) {
	const [state, setState] = useState<HistoryState<ClipEdit>>(() =>
		createHistoryState(initial),
	);

	const preview = useCallback((updater: (edit: ClipEdit) => ClipEdit) => {
		setState((current) => historyPreview(current, updater));
	}, []);

	const flush = useCallback(() => {
		setState((current) => historyFlush(current));
	}, []);

	const apply = useCallback((updater: (edit: ClipEdit) => ClipEdit) => {
		setState((current) => historyApply(current, updater));
	}, []);

	const undo = useCallback(() => {
		setState((current) => historyUndo(current));
	}, []);

	const redo = useCallback(() => {
		setState((current) => historyRedo(current));
	}, []);

	const reset = useCallback((edit: ClipEdit) => {
		setState(createHistoryState(edit));
	}, []);

	return {
		edit: state.present,
		preview,
		flush,
		apply,
		undo,
		redo,
		reset,
		canUndo: state.past.length > 0,
		canRedo: state.future.length > 0,
	};
}
