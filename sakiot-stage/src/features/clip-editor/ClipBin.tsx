import type { TouchEvent as ReactTouchEvent } from "react";
import { useEffect, useRef, useState } from "react";
import type { ClipData } from "../../app/apiSlice";
import { cn, Spinner, TextField } from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";

export function ClipBin(props: {
	clips: ClipData[];
	loadingClips: ReadonlyMap<string, boolean>;
	onAdd: (clip: ClipData) => void;
	onDragStart?: () => void;
	onDragEnd?: () => void;
	onTouchDragStart?: (clip: ClipData, clientX: number, clientY: number) => void;
	onTouchDragMove?: (clip: ClipData, clientX: number, clientY: number) => void;
	onTouchDrop?: (clip: ClipData, clientX: number, clientY: number) => void;
	onTouchDragCancel?: () => void;
	tapToAdd?: boolean;
	disableNativeDrag?: boolean;
}) {
	const [search, setSearch] = useState("");
	const query = search.trim().toLowerCase();
	const filtered = query
		? props.clips.filter((clip) =>
				(clip.name ?? clip.clip_id).toLowerCase().includes(query),
			)
		: props.clips;

	return (
		<aside
			aria-label="Source clips"
			data-testid="clip-source-bin"
			className={cn(
				"w-70 flex-none flex flex-col min-h-0 border-r border-ui-border",
				props.tapToAdd ? "h-full" : "h-auto",
			)}
		>
			<div className="p-3 pb-2">
				<TextField
					aria-label="Search clips"
					placeholder="Search clips"
					value={search}
					onChange={(value) => setSearch(value)}
				/>
				<span className="text-muted text-xs leading-5 px-1">
					{props.tapToAdd
						? "Tap to append, or hold and drag onto a track"
						: "Drag onto a track, or double-click to append"}
				</span>
			</div>
			<div className="flex-1 min-h-0 overflow-y-auto overflow-x-hidden">
				<div className="flex flex-col gap-1 px-2 pb-2">
					{filtered.map((clip) => (
						<ClipBinItem
							key={clip.clip_id}
							clip={clip}
							loading={props.loadingClips.get(clip.clip_id) === true}
							onAdd={props.onAdd}
							onDragStart={props.onDragStart}
							onDragEnd={props.onDragEnd}
							onTouchDragStart={props.onTouchDragStart}
							onTouchDragMove={props.onTouchDragMove}
							onTouchDrop={props.onTouchDrop}
							onTouchDragCancel={props.onTouchDragCancel}
							tapToAdd={props.tapToAdd}
							disableNativeDrag={props.disableNativeDrag}
						/>
					))}
					{filtered.length === 0 && (
						<p className="text-muted text-sm p-4">No clips match.</p>
					)}
				</div>
			</div>
		</aside>
	);
}

export interface BinDragPayload {
	clipId: string;
	lengthSec: number;
}

/**
 * The clip being dragged, captured at dragstart. dataTransfer.getData is
 * unreliable during dragover in some browsers, so the timeline reads this
 * instead; the dataTransfer copy stays for browsers that need payload data
 * to allow the drop.
 */
export const pendingBinDrag: { payload: BinDragPayload | null } = {
	payload: null,
};

function ClipBinItem(props: {
	clip: ClipData;
	loading: boolean;
	onAdd: (clip: ClipData) => void;
	onDragStart?: () => void;
	onDragEnd?: () => void;
	onTouchDragStart?: (clip: ClipData, clientX: number, clientY: number) => void;
	onTouchDragMove?: (clip: ClipData, clientX: number, clientY: number) => void;
	onTouchDrop?: (clip: ClipData, clientX: number, clientY: number) => void;
	onTouchDragCancel?: () => void;
	tapToAdd?: boolean;
	disableNativeDrag?: boolean;
}) {
	const touchRef = useRef<{
		identifier: number;
		startX: number;
		startY: number;
		clientX: number;
		clientY: number;
		active: boolean;
		moved: boolean;
	} | null>(null);
	const holdTimerRef = useRef<number | null>(null);
	const removeTouchListenersRef = useRef<(() => void) | null>(null);
	const ignoreClickUntilRef = useRef(0);

	const clearHoldTimer = () => {
		if (holdTimerRef.current === null) return;
		window.clearTimeout(holdTimerRef.current);
		holdTimerRef.current = null;
	};

	const finishTouchGesture = () => {
		clearHoldTimer();
		removeTouchListenersRef.current?.();
		removeTouchListenersRef.current = null;
		touchRef.current = null;
	};

	useEffect(
		() => () => {
			if (holdTimerRef.current !== null) {
				window.clearTimeout(holdTimerRef.current);
			}
			removeTouchListenersRef.current?.();
		},
		[],
	);

	const beginTouchHold = (event: ReactTouchEvent<HTMLElement>) => {
		if (!props.tapToAdd || event.touches.length !== 1) return;
		finishTouchGesture();
		const touch = event.touches[0];
		if (!touch) return;
		touchRef.current = {
			identifier: touch.identifier,
			startX: touch.clientX,
			startY: touch.clientY,
			clientX: touch.clientX,
			clientY: touch.clientY,
			active: false,
			moved: false,
		};

		const touchFor = (touches: TouchList, identifier: number) =>
			Array.from(touches).find(
				(candidate) => candidate.identifier === identifier,
			);

		const handleTouchMove = (moveEvent: TouchEvent) => {
			const gesture = touchRef.current;
			if (!gesture) return;
			const movedTouch = touchFor(moveEvent.touches, gesture.identifier);
			if (!movedTouch) return;
			gesture.clientX = movedTouch.clientX;
			gesture.clientY = movedTouch.clientY;
			const distance = Math.hypot(
				movedTouch.clientX - gesture.startX,
				movedTouch.clientY - gesture.startY,
			);
			if (!gesture.active) {
				if (distance > 10) finishTouchGesture();
				return;
			}
			moveEvent.preventDefault();
			gesture.moved = gesture.moved || distance > 12;
			props.onTouchDragMove?.(
				props.clip,
				movedTouch.clientX,
				movedTouch.clientY,
			);
		};

		const handleTouchEnd = (endEvent: TouchEvent) => {
			const gesture = touchRef.current;
			if (!gesture) return;
			const endedTouch = touchFor(endEvent.changedTouches, gesture.identifier);
			if (!endedTouch) return;
			if (gesture.active) {
				endEvent.preventDefault();
				ignoreClickUntilRef.current = Date.now() + 700;
				pendingBinDrag.payload = null;
				if (gesture.moved) {
					props.onTouchDrop?.(
						props.clip,
						endedTouch.clientX,
						endedTouch.clientY,
					);
				} else {
					props.onTouchDragCancel?.();
				}
			}
			finishTouchGesture();
		};

		const handleTouchCancel = () => {
			if (touchRef.current?.active) {
				pendingBinDrag.payload = null;
				props.onTouchDragCancel?.();
			}
			finishTouchGesture();
		};

		window.addEventListener("touchmove", handleTouchMove, { passive: false });
		window.addEventListener("touchend", handleTouchEnd, { passive: false });
		window.addEventListener("touchcancel", handleTouchCancel);
		removeTouchListenersRef.current = () => {
			window.removeEventListener("touchmove", handleTouchMove);
			window.removeEventListener("touchend", handleTouchEnd);
			window.removeEventListener("touchcancel", handleTouchCancel);
		};

		holdTimerRef.current = window.setTimeout(() => {
			const gesture = touchRef.current;
			if (!gesture) return;
			gesture.active = true;
			pendingBinDrag.payload = {
				clipId: props.clip.clip_id,
				lengthSec: props.clip.length ?? 0,
			};
			props.onTouchDragStart?.(props.clip, gesture.clientX, gesture.clientY);
		}, 300);
	};

	return (
		<button
			type="button"
			draggable={!props.disableNativeDrag}
			onTouchStart={beginTouchHold}
			onContextMenu={(event) => {
				if (props.tapToAdd) event.preventDefault();
			}}
			onDragStart={(event) => {
				pendingBinDrag.payload = {
					clipId: props.clip.clip_id,
					lengthSec: props.clip.length ?? 0,
				};
				event.dataTransfer.setData(
					"application/json",
					JSON.stringify(pendingBinDrag.payload),
				);
				event.dataTransfer.effectAllowed = "copy";
				const chip = document.createElement("div");
				chip.textContent = props.clip.name || "Clip";
				chip.style.cssText =
					"position:absolute;top:-1000px;left:-1000px;padding:4px 8px;" +
					"border-radius:6px;border:1px dashed #38bdf8;" +
					"background:rgba(15,23,42,0.9);color:#e2e8f0;" +
					"font-size:12px;white-space:nowrap;";
				document.body.appendChild(chip);
				event.dataTransfer.setDragImage(
					chip,
					chip.offsetWidth / 2,
					chip.offsetHeight + 6,
				);
				requestAnimationFrame(() => chip.remove());
				props.onDragStart?.();
			}}
			onDragEnd={() => {
				pendingBinDrag.payload = null;
				props.onDragEnd?.();
			}}
			onDoubleClick={() => {
				if (!props.tapToAdd) props.onAdd(props.clip);
			}}
			className="w-full justify-start text-left px-3 py-1.5 mb-1.5 border border-ui-border [border-radius:1px] bg-surface-raised [touch-action:pan-y] select-none last:mb-0 hover:[background-color:color-mix(in_srgb,_var(--color-surface-raised)_82%,_white)] hover:border-primary-strong"
			onClick={(event) => {
				if (Date.now() < ignoreClickUntilRef.current) {
					event.preventDefault();
					return;
				}
				if (props.tapToAdd) props.onAdd(props.clip);
			}}
		>
			<div className="my-0">
				{props.clip.name || "Unnamed clip"}
				<span className="block text-xs text-muted">
					{formatDuration(props.clip.length ?? 0)}
				</span>
			</div>
			{props.loading && <Spinner />}
		</button>
	);
}
