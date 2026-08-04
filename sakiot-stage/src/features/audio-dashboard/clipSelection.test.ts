import { describe, expect, test } from "bun:test";
import {
	advanceFineDrag,
	applyEdge,
	applyEdgeWithinWindow,
	beginFineDrag,
	canSetSelectionEdge,
	changedEdge,
	constrainFineDragToLens,
	constrainFineDragToWindow,
	defaultDetailWindowMs,
	fineDragMultiplier,
	moveSelection,
	moveSelectionWithinWindow,
	nearestSelectionEdge,
	nudgeEdge,
	panWindowToInclude,
	precisionLensWindowMs,
	precisionZoneBounds,
	rollingEdgeStrength,
	rollingRulerWindow,
	selectionFitsWindow,
	selectionShiftedAsBand,
	selectionWindowGeometry,
	setNearestSelectionEdge,
	shiftWindow,
	transformSelectionWithWindow,
	windowAround,
	windowCenter,
	windowForSelection,
	windowFraction,
	windowMsPerPixel,
	zoomDetailWindow,
} from "./clipSelection";

const SESSION_MS = 22_484_000; // 6h14m44s, the recording that motivated this.

describe("defaultDetailWindowMs", () => {
	test("uses ten percent of the session", () => {
		expect(defaultDetailWindowMs(SESSION_MS)).toBe(2_248_400);
		expect(defaultDetailWindowMs(30_000)).toBe(3_000);
	});

	test("handles invalid and negative durations", () => {
		expect(defaultDetailWindowMs(-10_000)).toBe(0);
		expect(defaultDetailWindowMs(Number.NaN)).toBe(0);
	});
});

describe("windowForSelection", () => {
	test("centres the window on the selection", () => {
		expect(windowForSelection([100_000, 110_000], 60_000, SESSION_MS)).toEqual({
			startMs: 75_000,
			endMs: 135_000,
		});
	});

	test("keeps the window inside the recording at the start", () => {
		expect(windowForSelection([0, 2_000], 60_000, SESSION_MS)).toEqual({
			startMs: 0,
			endMs: 60_000,
		});
	});

	test("keeps the window inside the recording at the end", () => {
		expect(
			windowForSelection([SESSION_MS - 1_000, SESSION_MS], 60_000, SESSION_MS),
		).toEqual({ startMs: SESSION_MS - 60_000, endMs: SESSION_MS });
	});

	test("never hides part of the selection", () => {
		const window = windowForSelection([0, 600_000], 60_000, SESSION_MS);

		expect(window.endMs - window.startMs).toBe(600_000);
	});

	test("falls back to the whole recording when it is shorter than the window", () => {
		expect(windowForSelection([0, 1_000], 60_000, 30_000)).toEqual({
			startMs: 0,
			endMs: 30_000,
		});
	});
});

describe("panWindowToInclude", () => {
	const window = { startMs: 100_000, endMs: 160_000 };

	test("leaves a window that already shows the value", () => {
		expect(panWindowToInclude(window, 130_000, SESSION_MS)).toEqual(window);
	});

	test("scrolls left by the least amount when the value falls behind", () => {
		expect(panWindowToInclude(window, 99_000, SESSION_MS)).toEqual({
			startMs: 94_200,
			endMs: 154_200,
		});
	});

	test("scrolls right by the least amount when the value runs ahead", () => {
		expect(panWindowToInclude(window, 161_000, SESSION_MS)).toEqual({
			startMs: 105_800,
			endMs: 165_800,
		});
	});

	test("stops at the end of the recording", () => {
		expect(
			panWindowToInclude(
				{ startMs: SESSION_MS - 60_000, endMs: SESSION_MS },
				SESSION_MS,
				SESSION_MS,
			),
		).toEqual({ startMs: SESSION_MS - 60_000, endMs: SESSION_MS });
	});
});

describe("windowAround", () => {
	test("centres on the instant", () => {
		expect(windowAround(100_000, 60_000, SESSION_MS)).toEqual({
			startMs: 70_000,
			endMs: 130_000,
		});
	});

	test("clamps at both ends of the recording", () => {
		expect(windowAround(0, 60_000, SESSION_MS)).toEqual({
			startMs: 0,
			endMs: 60_000,
		});
		expect(windowAround(SESSION_MS, 60_000, SESSION_MS)).toEqual({
			startMs: SESSION_MS - 60_000,
			endMs: SESSION_MS,
		});
	});
});

describe("shiftWindow", () => {
	const window = { startMs: 100_000, endMs: 160_000 };

	test("slides the window without changing its width", () => {
		expect(shiftWindow(window, 25_000, SESSION_MS)).toEqual({
			startMs: 125_000,
			endMs: 185_000,
		});
	});

	test("stops at either end of the recording", () => {
		expect(shiftWindow(window, -200_000, SESSION_MS)).toEqual({
			startMs: 0,
			endMs: 60_000,
		});
		expect(shiftWindow(window, SESSION_MS, SESSION_MS)).toEqual({
			startMs: SESSION_MS - 60_000,
			endMs: SESSION_MS,
		});
	});
});

describe("transformSelectionWithWindow", () => {
	test("moves the clip range with a panned session window", () => {
		expect(
			transformSelectionWithWindow(
				[110_000, 125_000],
				{ startMs: 100_000, endMs: 160_000 },
				{ startMs: 250_000, endMs: 310_000 },
			),
		).toEqual([260_000, 275_000]);
	});

	test("scales the clip range proportionally when zooming", () => {
		expect(
			transformSelectionWithWindow(
				[115_000, 145_000],
				{ startMs: 100_000, endMs: 160_000 },
				{ startMs: 115_000, endMs: 145_000 },
			),
		).toEqual([122_500, 137_500]);
		expect(
			transformSelectionWithWindow(
				[122_500, 137_500],
				{ startMs: 115_000, endMs: 145_000 },
				{ startMs: 100_000, endMs: 160_000 },
			),
		).toEqual([115_000, 145_000]);
	});

	test("repairs an out-of-window range before moving it", () => {
		expect(
			transformSelectionWithWindow(
				[60_000, 75_000],
				{ startMs: 100_000, endMs: 160_000 },
				{ startMs: 200_000, endMs: 260_000 },
			),
		).toEqual([200_000, 215_000]);
	});

	test("fits an oversized range to the destination window", () => {
		expect(
			transformSelectionWithWindow(
				[80_000, 180_000],
				{ startMs: 100_000, endMs: 160_000 },
				{ startMs: 220_000, endMs: 250_000 },
			),
		).toEqual([220_000, 250_000]);
	});
});

describe("changedEdge", () => {
	test("names the edge that moved", () => {
		expect(changedEdge([0, 10_000], [500, 10_000])).toBe("start");
		expect(changedEdge([0, 10_000], [0, 9_000])).toBe("end");
	});

	test("prefers the start when a move shifted both", () => {
		expect(changedEdge([0, 10_000], [500, 10_500])).toBe("start");
	});

	test("reports nothing when the selection held still", () => {
		expect(changedEdge([0, 10_000], [0, 10_000])).toBeNull();
	});
});

describe("selectionShiftedAsBand", () => {
	test("recognises both edges moving by the same amount", () => {
		expect(selectionShiftedAsBand([10_000, 20_000], [15_000, 25_000])).toBe(
			true,
		);
	});

	test("rejects edge edits, resizes, and stationary selections", () => {
		expect(selectionShiftedAsBand([10_000, 20_000], [11_000, 20_000])).toBe(
			false,
		);
		expect(selectionShiftedAsBand([10_000, 20_000], [11_000, 22_000])).toBe(
			false,
		);
		expect(selectionShiftedAsBand([10_000, 20_000], [10_000, 20_000])).toBe(
			false,
		);
	});
});

describe("selectionFitsWindow", () => {
	test("tells a clip-sized selection from a whole-session one", () => {
		expect(selectionFitsWindow([0, 12_000], 60_000)).toBe(true);
		expect(selectionFitsWindow([0, SESSION_MS], 60_000)).toBe(false);
	});
});

describe("window geometry", () => {
	test("maps an instant to its fraction of the window", () => {
		expect(windowFraction(130_000, { startMs: 100_000, endMs: 160_000 })).toBe(
			0.5,
		);
	});

	test("reports the resolution a drag works at", () => {
		expect(windowMsPerPixel({ startMs: 0, endMs: 60_000 }, 1_200)).toBeCloseTo(
			50,
			10,
		);
	});

	test("reports the midpoint a zoom keeps in place", () => {
		expect(windowCenter({ startMs: 100_000, endMs: 160_000 })).toBe(130_000);
	});

	test("zooms through the presets and stops at the ends", () => {
		expect(zoomDetailWindow(60_000, 1)).toBe(30_000);
		expect(zoomDetailWindow(60_000, -1)).toBe(300_000);
		expect(zoomDetailWindow(5_000, 1)).toBe(5_000);
		expect(zoomDetailWindow(1_800_000, -1)).toBe(1_800_000);
	});

	test("keeps a duration-relative default as an extra zoom stop", () => {
		const defaultWindowMs = defaultDetailWindowMs(SESSION_MS);

		expect(zoomDetailWindow(defaultWindowMs, 1, defaultWindowMs)).toBe(
			1_800_000,
		);
		expect(zoomDetailWindow(1_800_000, -1, defaultWindowMs)).toBe(
			defaultWindowMs,
		);
	});

	test("clips a long band without inventing handles at the window edges", () => {
		expect(
			selectionWindowGeometry([0, SESSION_MS], {
				startMs: 100_000,
				endMs: 160_000,
			}),
		).toEqual({
			startFraction: 0,
			endFraction: 1,
			startHandleVisible: false,
			endHandleVisible: false,
			overlaps: true,
		});
	});

	test("keeps real endpoints draggable when they are inside the window", () => {
		expect(
			selectionWindowGeometry([110_000, 140_000], {
				startMs: 100_000,
				endMs: 160_000,
			}),
		).toEqual({
			startFraction: 1 / 6,
			endFraction: 2 / 3,
			startHandleVisible: true,
			endHandleVisible: true,
			overlaps: true,
		});
	});
});

describe("rolling fine ruler", () => {
	const allowed = { startMs: 95_000, endMs: 105_000 };

	test("places the value beneath the pointer inside the allowed range", () => {
		const ruler = rollingRulerWindow(allowed, 100_000, 0.9);

		expect(ruler).toEqual({ startMs: 95_500, endMs: 100_500 });
		expect(windowFraction(100_000, ruler)).toBeCloseTo(0.9, 10);
	});

	test("rolls to either hard limit without leaving the allowed range", () => {
		expect(rollingRulerWindow(allowed, 95_000, 0)).toEqual({
			startMs: 95_000,
			endMs: 100_000,
		});
		expect(rollingRulerWindow(allowed, 105_000, 1)).toEqual({
			startMs: 100_000,
			endMs: 105_000,
		});
	});

	test("can zoom the visible ruler without changing its allowed range", () => {
		expect(rollingRulerWindow(allowed, 100_000, 0.5, 1_000)).toEqual({
			startMs: 99_500,
			endMs: 100_500,
		});
	});

	test("reports signed pressure only inside the edge zones", () => {
		expect(rollingEdgeStrength(0, 0, 1_000, 40)).toBe(-1);
		expect(rollingEdgeStrength(20, 0, 1_000, 40)).toBe(-0.5);
		expect(rollingEdgeStrength(500, 0, 1_000, 40)).toBe(0);
		expect(rollingEdgeStrength(980, 0, 1_000, 40)).toBe(0.5);
		expect(rollingEdgeStrength(1_000, 0, 1_000, 40)).toBe(1);
	});
});

describe("applyEdge", () => {
	test("moves the requested edge", () => {
		expect(applyEdge([10_000, 20_000], "start", 12_000, SESSION_MS)).toEqual([
			12_000, 20_000,
		]);
		expect(applyEdge([10_000, 20_000], "end", 18_000, SESSION_MS)).toEqual([
			10_000, 18_000,
		]);
	});

	test("does not let the edges cross", () => {
		expect(applyEdge([10_000, 20_000], "start", 25_000, SESSION_MS)).toEqual([
			20_000, 20_000,
		]);
		expect(applyEdge([10_000, 20_000], "end", 5_000, SESSION_MS)).toEqual([
			10_000, 10_000,
		]);
	});

	test("clamps to the recording", () => {
		expect(applyEdge([10_000, 20_000], "start", -5_000, SESSION_MS)).toEqual([
			0, 20_000,
		]);
		expect(
			applyEdge([10_000, 20_000], "end", SESSION_MS + 5_000, SESSION_MS),
		).toEqual([10_000, SESSION_MS]);
	});

	test("allows ranges far longer than a clip, for downloads", () => {
		expect(applyEdge([0, 1_000], "end", SESSION_MS, SESSION_MS)).toEqual([
			0,
			SESSION_MS,
		]);
	});

	test("nudges by a delta", () => {
		expect(nudgeEdge([10_000, 20_000], "end", -100, SESSION_MS)).toEqual([
			10_000, 19_900,
		]);
	});
});

describe("applyEdgeWithinWindow", () => {
	const window = { startMs: 100_000, endMs: 160_000 };

	test("stops the left handle at the clip-window start", () => {
		expect(
			applyEdgeWithinWindow(
				[110_000, 125_000],
				"start",
				-1_000_000_000,
				SESSION_MS,
				window,
			),
		).toEqual([100_000, 125_000]);
	});

	test("stops the right handle at the clip-window end", () => {
		expect(
			applyEdgeWithinWindow(
				[110_000, 125_000],
				"end",
				1_000_000_000,
				SESSION_MS,
				window,
			),
		).toEqual([110_000, 160_000]);
	});
});

describe("direction-aware edge controls", () => {
	const selection: [number, number] = [10_000, 20_000];

	test("chooses the nearest edge inside and outside the selection", () => {
		expect(nearestSelectionEdge(selection, 5_000)).toBe("start");
		expect(nearestSelectionEdge(selection, 12_000)).toBe("start");
		expect(nearestSelectionEdge(selection, 18_000)).toBe("end");
		expect(nearestSelectionEdge(selection, 25_000)).toBe("end");
	});

	test("does not let explicit edge actions collapse or cross", () => {
		expect(canSetSelectionEdge(selection, "start", 19_999)).toBe(true);
		expect(canSetSelectionEdge(selection, "start", 20_000)).toBe(false);
		expect(canSetSelectionEdge(selection, "end", 10_001)).toBe(true);
		expect(canSetSelectionEdge(selection, "end", 10_000)).toBe(false);
	});

	test("moves the closest edge to a position inside the selection", () => {
		expect(setNearestSelectionEdge(selection, 12_000, SESSION_MS)).toEqual([
			12_000, 20_000,
		]);
		expect(setNearestSelectionEdge(selection, 18_000, SESSION_MS)).toEqual([
			10_000, 18_000,
		]);
	});
});

describe("moveSelection", () => {
	test("slides both edges together", () => {
		expect(moveSelection([10_000, 20_000], 5_000, SESSION_MS)).toEqual([
			15_000, 25_000,
		]);
	});

	test("keeps the length when it hits the start", () => {
		expect(moveSelection([10_000, 20_000], -50_000, SESSION_MS)).toEqual([
			0, 10_000,
		]);
	});

	test("keeps the length when it hits the end", () => {
		expect(
			moveSelection([SESSION_MS - 10_000, SESSION_MS], 50_000, SESSION_MS),
		).toEqual([SESSION_MS - 10_000, SESSION_MS]);
	});
});

describe("moveSelectionWithinWindow", () => {
	const window = { startMs: 100_000, endMs: 160_000 };

	test("slides both edges inside the visible clip window", () => {
		expect(
			moveSelectionWithinWindow([110_000, 125_000], 10_000, window),
		).toEqual([120_000, 135_000]);
	});

	test("stops when the left edge reaches the window start", () => {
		expect(
			moveSelectionWithinWindow([110_000, 125_000], -50_000, window),
		).toEqual([100_000, 115_000]);
	});

	test("stops when the right edge reaches the window end", () => {
		expect(
			moveSelectionWithinWindow([110_000, 125_000], 50_000, window),
		).toEqual([145_000, 160_000]);
	});

	test("stays pinned during arbitrarily far off-screen pointer travel", () => {
		const selection: [number, number] = [110_000, 125_000];
		expect(
			moveSelectionWithinWindow(selection, -1_000_000_000, window),
		).toEqual([100_000, 115_000]);
		expect(moveSelectionWithinWindow(selection, 1_000_000_000, window)).toEqual(
			[145_000, 160_000],
		);
	});

	test("does not move a selection wider than the clip window", () => {
		const selection: [number, number] = [90_000, 170_000];
		expect(moveSelectionWithinWindow(selection, 5_000, window)).toBe(selection);
	});
});

describe("fineDragMultiplier", () => {
	test("stays coarse near the handle", () => {
		expect(fineDragMultiplier(0)).toBe(1);
		expect(fineDragMultiplier(-31)).toBe(1);
	});

	test("uses wide precision zones only above the handle", () => {
		expect(fineDragMultiplier(-32)).toBe(10);
		expect(fineDragMultiplier(-159)).toBe(10);
		expect(fineDragMultiplier(-160)).toBe(100);
		expect(fineDragMultiplier(400)).toBe(1);
	});
});

describe("precisionLensWindowMs", () => {
	test("keeps useful context at both fine levels", () => {
		expect(precisionLensWindowMs(60_000, 10)).toBe(10_000);
		expect(precisionLensWindowMs(60_000, 100)).toBe(3_000);
		expect(precisionLensWindowMs(300_000, 10)).toBe(30_000);
	});

	test("prevents a fine drag drifting beyond the visible fades", () => {
		const drag = {
			...beginFineDrag(500, 100_000),
			multiplier: 10,
			lensAnchorMs: 100_000,
			valueMs: 120_000,
		};

		expect(constrainFineDragToLens(drag, 60_000, SESSION_MS).valueMs).toBe(
			105_000,
		);
	});

	test("keeps a playhead inside its clip window", () => {
		const drag = beginFineDrag(500, 100_000);
		const window = { startMs: 95_000, endMs: 105_000 };

		expect(
			constrainFineDragToWindow({ ...drag, valueMs: 90_000 }, window).valueMs,
		).toBe(95_000);
		expect(
			constrainFineDragToWindow({ ...drag, valueMs: 110_000 }, window).valueMs,
		).toBe(105_000);
	});
});

describe("precisionZoneBounds", () => {
	test("describes the normal, fine, and ultra vertical bands", () => {
		expect(precisionZoneBounds(500, 1, 1_000)).toEqual({
			topPx: 468,
			bottomPx: 1_000,
		});
		expect(precisionZoneBounds(500, 10, 1_000)).toEqual({
			topPx: 340,
			bottomPx: 468,
		});
		expect(precisionZoneBounds(500, 100, 1_000)).toEqual({
			topPx: 0,
			bottomPx: 340,
		});
	});
});

describe("advanceFineDrag", () => {
	const msPerPx = 50; // a 60s window across 1200px

	test("tracks the pointer at full rate near the handle", () => {
		const drag = beginFineDrag(500, 100_000);

		const moved = advanceFineDrag(drag, { xPx: 600, dyPx: 0, msPerPx });

		expect(moved.valueMs).toBe(105_000);
	});

	test("moves ten times slower once the pointer pulls away", () => {
		const drag = beginFineDrag(500, 100_000);

		const moved = advanceFineDrag(drag, { xPx: 600, dyPx: -40, msPerPx });

		expect(moved.valueMs).toBe(100_500);
	});

	test("changes precision without jumping the current value", () => {
		let drag = beginFineDrag(500, 100_000);
		const coarse = advanceFineDrag(drag, { xPx: 600, dyPx: 0, msPerPx });
		drag = coarse.state;

		const atThreshold = advanceFineDrag(drag, {
			xPx: 600,
			dyPx: -40,
			msPerPx,
		});

		expect(atThreshold.valueMs).toBe(coarse.valueMs);
		expect(atThreshold.state.multiplier).toBe(10);

		const finer = advanceFineDrag(atThreshold.state, {
			xPx: 620,
			dyPx: -40,
			msPerPx,
		});

		expect(finer.valueMs).toBe(105_100);
	});

	test("returns to full rate when the pointer comes back", () => {
		let drag = beginFineDrag(500, 100_000);
		drag = advanceFineDrag(drag, { xPx: 600, dyPx: -200, msPerPx }).state;

		const back = advanceFineDrag(drag, { xPx: 600, dyPx: 0, msPerPx });

		expect(back.state.multiplier).toBe(1);
		expect(back.valueMs).toBe(100_050);
	});

	test("anchors each precision level on the unchanged current value", () => {
		let drag = beginFineDrag(500, 100_000);
		drag = advanceFineDrag(drag, {
			xPx: 520,
			dyPx: 0,
			msPerPx,
		}).state;
		const entered = advanceFineDrag(drag, {
			xPx: 520,
			dyPx: -40,
			msPerPx,
		});
		drag = entered.state;
		const enteredUltra = advanceFineDrag(drag, {
			xPx: 540,
			dyPx: -160,
			msPerPx,
		});
		const returnedToFine = advanceFineDrag(enteredUltra.state, {
			xPx: 540,
			dyPx: -40,
			msPerPx,
		});

		expect(entered.state.lensAnchorMs).toBe(entered.valueMs);
		expect(enteredUltra.state.lensAnchorMs).toBe(enteredUltra.valueMs);
		expect(returnedToFine.state.lensAnchorMs).toBe(returnedToFine.valueMs);
		expect(entered.state.originMs).toBe(100_000);
		expect(enteredUltra.state.originMs).toBe(100_000);
	});
});
