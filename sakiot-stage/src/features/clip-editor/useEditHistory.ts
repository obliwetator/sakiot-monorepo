import { useCallback, useEffect, useRef, useState } from "react";
import type { ClipEdit } from "./model";

const HISTORY_LIMIT = 100;

/**
 * Undo/redo with gesture batching: `preview` mutates a pending draft, `flush`
 * folds every preview since the last flush into one history step. Timeline
 * drags preview on every pointermove and flush on pointerup, so a whole drag
 * is one undo step.
 */
export function useEditHistory(initial: ClipEdit) {
	const editRef = useRef(initial);
	const baseRef = useRef(initial);
	const [past, setPast] = useState<ClipEdit[]>([]);
	const [present, setPresent] = useState(initial);
	const [future, setFuture] = useState<ClipEdit[]>([]);

	useEffect(() => {
		editRef.current = present;
	}, [present]);

	const preview = useCallback((updater: (edit: ClipEdit) => ClipEdit) => {
		const next = updater(editRef.current);
		editRef.current = next;
		setPresent(next);
	}, []);

	const flush = useCallback(() => {
		if (baseRef.current === editRef.current) return;
		setPast((previous) => [...previous, baseRef.current].slice(-HISTORY_LIMIT));
		baseRef.current = editRef.current;
		setFuture([]);
	}, []);

	const apply = useCallback((updater: (edit: ClipEdit) => ClipEdit) => {
		const current = editRef.current;
		const next = updater(current);
		if (next === current) return;
		setPast((previous) => [...previous, current].slice(-HISTORY_LIMIT));
		baseRef.current = next;
		editRef.current = next;
		setPresent(next);
		setFuture([]);
	}, []);

	const undo = useCallback(() => {
		if (past.length === 0) return;
		const current = editRef.current;
		const last = past[past.length - 1];
		editRef.current = last;
		baseRef.current = last;
		setPresent(last);
		setPast(past.slice(0, -1));
		setFuture([current, ...future].slice(0, HISTORY_LIMIT));
	}, [past, future]);

	const redo = useCallback(() => {
		if (future.length === 0) return;
		const current = editRef.current;
		const [first, ...rest] = future;
		editRef.current = first;
		baseRef.current = first;
		setPresent(first);
		setFuture(rest);
		setPast([...past, current].slice(-HISTORY_LIMIT));
	}, [past, future]);

	const reset = useCallback((edit: ClipEdit) => {
		editRef.current = edit;
		baseRef.current = edit;
		setPast([]);
		setFuture([]);
		setPresent(edit);
	}, []);

	return {
		edit: present,
		preview,
		flush,
		apply,
		undo,
		redo,
		reset,
		canUndo: past.length > 0,
		canRedo: future.length > 0,
	};
}
