export interface HistoryState<Edit> {
	past: Edit[];
	present: Edit;
	future: Edit[];
	/** The state the current gesture started from; folded into past on flush. */
	base: Edit;
}

export const HISTORY_LIMIT = 100;

export function createHistoryState<Edit>(initial: Edit): HistoryState<Edit> {
	return { past: [], present: initial, future: [], base: initial };
}

/**
 * Pure undo/redo transitions. The hook stores the whole state in one useState;
 * every transition returns a new state, so gestures (`preview` + `flush`) and
 * discrete edits (`apply`) compose the same way and are trivially testable.
 */
export function historyPreview<Edit>(
	state: HistoryState<Edit>,
	updater: (edit: Edit) => Edit,
): HistoryState<Edit> {
	const next = updater(state.present);
	if (next === state.present) return state;
	return { ...state, present: next };
}

export function historyFlush<Edit>(
	state: HistoryState<Edit>,
): HistoryState<Edit> {
	if (state.base === state.present) return state;
	return {
		...state,
		past: [...state.past, state.base].slice(-HISTORY_LIMIT),
		base: state.present,
		future: [],
	};
}

export function historyApply<Edit>(
	state: HistoryState<Edit>,
	updater: (edit: Edit) => Edit,
): HistoryState<Edit> {
	const next = updater(state.present);
	if (next === state.present) return state;
	return {
		...state,
		past: [...state.past, state.present].slice(-HISTORY_LIMIT),
		present: next,
		base: next,
		future: [],
	};
}

export function historyUndo<Edit>(
	state: HistoryState<Edit>,
): HistoryState<Edit> {
	if (state.past.length === 0) return state;
	const last = state.past[state.past.length - 1];
	return {
		...state,
		past: state.past.slice(0, -1),
		present: last,
		future: [state.present, ...state.future].slice(0, HISTORY_LIMIT),
		base: last,
	};
}

export function historyRedo<Edit>(
	state: HistoryState<Edit>,
): HistoryState<Edit> {
	if (state.future.length === 0) return state;
	const [first, ...rest] = state.future;
	return {
		...state,
		past: [...state.past, state.present].slice(-HISTORY_LIMIT),
		present: first,
		future: rest,
		base: first,
	};
}
