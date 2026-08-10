import { useCallback, useEffect, useRef, useState } from "react";

export interface PointerDragSnapshot<G> {
	/** The state the gesture started from; kept untouched until commit. */
	origin: G;
	/** The live preview following the pointer; `compute` replaces it. */
	ghost: G;
	pointerX: number;
	pointerY: number;
}

export interface PointerDragOptions<G> {
	/** Pure: derive the next ghost from the current one and the pointer. */
	compute: (ghost: G, event: PointerEvent) => G;
	/** Called once on pointerup with the final ghost and pointer position. */
	onCommit: (snapshot: PointerDragSnapshot<G>) => void;
	/** Called when the gesture is cancelled without committing (escape, scroll…). */
	onCancel?: (snapshot: PointerDragSnapshot<G>) => void;
}

/**
 * Shared gesture core for pointer drags: pointerdown calls `begin`, then the
 * drag runs on window listeners (so fast mouse movement or leaving the
 * element never loses it), the caller's `compute` drives a ghost snapshot on
 * every move, and pointerup commits once. The origin stays untouched until
 * commit, so an invalid or cancelled release simply reverts.
 */
export function usePointerDrag<G>(options: PointerDragOptions<G>) {
	const [dragging, setDragging] = useState(false);
	const [snapshot, setSnapshot] = useState<PointerDragSnapshot<G> | null>(null);
	const snapshotRef = useRef<PointerDragSnapshot<G> | null>(null);
	const optionsRef = useRef(options);
	optionsRef.current = options;

	const begin = useCallback((origin: G, pointerX: number, pointerY: number) => {
		const next = { origin, ghost: origin, pointerX, pointerY };
		snapshotRef.current = next;
		setSnapshot(next);
		setDragging(true);
	}, []);

	const cancel = useCallback(() => {
		const current = snapshotRef.current;
		snapshotRef.current = null;
		setSnapshot(null);
		setDragging(false);
		if (current) optionsRef.current.onCancel?.(current);
	}, []);

	useEffect(() => {
		if (!dragging) return;
		const move = (event: PointerEvent) => {
			const current = snapshotRef.current;
			if (!current) return;
			const next = {
				...current,
				ghost: optionsRef.current.compute(current.ghost, event),
				pointerX: event.clientX,
				pointerY: event.clientY,
			};
			snapshotRef.current = next;
			setSnapshot(next);
		};
		const up = (event: PointerEvent) => {
			const current = snapshotRef.current;
			snapshotRef.current = null;
			setSnapshot(null);
			setDragging(false);
			if (current) {
				optionsRef.current.onCommit({
					...current,
					pointerX: event.clientX,
					pointerY: event.clientY,
				});
			}
		};
		const cancelDrag = () => cancel();
		// A gesture can be orphaned without a pointerup: the browser can steal
		// the pointer (losing capture), or the window can blur mid-drag. Both
		// leave the ghost behind, so the marquee would keep following the
		// mouse forever; treat them as a cancel. After a normal pointerup the
		// snapshot is already null, so the extra cancel is a no-op.
		const lostPointer = () => cancel();
		const windowBlur = () => cancel();
		window.addEventListener("pointermove", move);
		window.addEventListener("pointerup", up);
		window.addEventListener("pointercancel", cancelDrag);
		window.addEventListener("lostpointercapture", lostPointer);
		window.addEventListener("blur", windowBlur);
		return () => {
			window.removeEventListener("pointermove", move);
			window.removeEventListener("pointerup", up);
			window.removeEventListener("pointercancel", cancelDrag);
			window.removeEventListener("lostpointercapture", lostPointer);
			window.removeEventListener("blur", windowBlur);
		};
	}, [cancel, dragging]);

	return { snapshot, begin, cancel, isDragging: dragging };
}
