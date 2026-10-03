import { useEffect, useState } from "react";
import {
	type DraftAction,
	type DraftState,
	draftReducer,
	type Equal,
	viewDraft,
} from "./draftField";

export interface DraftField<T> {
	/** What the input shows: the draft while edited, else the saved value. */
	value: T | undefined;
	dirty: boolean;
	/** The newer saved value, when it changed while this field was edited. */
	savedUpdate: { value: T } | null;
	edit: (value: T) => void;
	/** Drop the draft and show the saved value. */
	takeSaved: () => void;
	/** Call after this field's draft was saved successfully. */
	markSaved: () => void;
}

/**
 * A settings field that survives refreshes; see `draftField.ts` for the
 * rules. `scope` (usually the guild id) resets the draft when it changes.
 */
export function useDraftField<T>(
	saved: T | undefined,
	scope: string,
	equal: Equal<T> = Object.is,
): DraftField<T> {
	const [state, setState] = useState<DraftState<T> | null>(null);
	const dispatch = (action: DraftAction<T>) =>
		setState((current) => draftReducer(current, action, equal));
	const view = viewDraft(state, saved, scope, equal);

	useEffect(() => {
		if (view.settled) {
			setState((current) => draftReducer(current, { type: "settled" }, equal));
		}
	}, [view.settled, equal]);

	return {
		value: view.value,
		dirty: view.dirty,
		savedUpdate: view.savedUpdate,
		edit: (value: T) => dispatch({ type: "edit", value, scope, saved }),
		takeSaved: () => dispatch({ type: "use-saved" }),
		markSaved: () => dispatch({ type: "saved" }),
	};
}

/** Order-insensitive equality for id lists (excluded channels). */
export function sameIds(a: readonly string[], b: readonly string[]): boolean {
	if (a.length !== b.length) return false;
	const set = new Set(a);
	return b.every((id) => set.has(id));
}
