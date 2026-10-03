/**
 * One settings field's draft, kept apart from the saved value so a refresh
 * (another admin's save, a realtime update) never overwrites what the user is
 * typing. Pure, so the rules are testable without rendering.
 *
 * - Untouched: the field shows the saved value and follows its changes.
 * - Edited: the field keeps the draft. If the saved value changes from what
 *   it was when editing began, the change is surfaced so the user can take
 *   the saved value or save theirs (last write wins).
 * - Saved: the draft is held until the saved value catches up with it, so the
 *   field never flashes the old value while the refetch is in flight.
 */
export interface DraftState<T> {
	/** The guild (or other scope) the draft belongs to; another scope resets. */
	scope: string;
	value: T;
	/** The saved value when editing began. */
	base: T | undefined;
	/** Set once this draft was saved: clear it when the saved value matches. */
	settleWhen?: T;
}

export type DraftAction<T> =
	| { type: "edit"; value: T; scope: string; saved: T | undefined }
	| { type: "use-saved" }
	| { type: "saved" }
	| { type: "settled" };

export type Equal<T> = (a: T, b: T) => boolean;

export function draftReducer<T>(
	state: DraftState<T> | null,
	action: DraftAction<T>,
	equal: Equal<T>,
): DraftState<T> | null {
	switch (action.type) {
		case "edit": {
			if (action.saved !== undefined && equal(action.value, action.saved)) {
				return null;
			}
			const continuing = state !== null && state.scope === action.scope;
			return {
				scope: action.scope,
				value: action.value,
				base: continuing ? state.base : action.saved,
			};
		}
		case "use-saved":
		case "settled":
			return null;
		case "saved":
			return state === null ? null : { ...state, settleWhen: state.value };
	}
}

export interface DraftView<T> {
	value: T | undefined;
	dirty: boolean;
	/** The newer saved value, when it changed while this field was edited. */
	savedUpdate: { value: T } | null;
	/** The saved value caught up with a saved draft: drop the draft. */
	settled: boolean;
}

export function viewDraft<T>(
	state: DraftState<T> | null,
	saved: T | undefined,
	scope: string,
	equal: Equal<T>,
): DraftView<T> {
	const draft = state !== null && state.scope === scope ? state : null;
	if (draft === null) {
		return { value: saved, dirty: false, savedUpdate: null, settled: false };
	}
	const settled =
		draft.settleWhen !== undefined &&
		saved !== undefined &&
		equal(saved, draft.settleWhen);
	const changedUnderneath =
		saved !== undefined &&
		draft.settleWhen === undefined &&
		(draft.base === undefined || !equal(draft.base, saved)) &&
		!equal(draft.value, saved);
	return {
		value: draft.value,
		dirty: draft.settleWhen === undefined,
		savedUpdate: changedUnderneath ? { value: saved } : null,
		settled,
	};
}
