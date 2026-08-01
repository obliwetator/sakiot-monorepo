import { describe, expect, test } from "bun:test";
import {
	advanceFineDrag,
	applyEdge,
	beginFineDrag,
	changedEdge,
	fineDragMultiplier,
	moveSelection,
	nudgeEdge,
	panWindowToInclude,
	selectionFitsWindow,
	windowAround,
	windowCenter,
	windowForSelection,
	windowFraction,
	windowMsPerPixel,
	zoomDetailWindow,
} from "./clipSelection";

const SESSION_MS = 22_484_000; // 6h14m44s, the recording that motivated this.

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

describe("fineDragMultiplier", () => {
	test("stays coarse near the handle", () => {
		expect(fineDragMultiplier(0)).toBe(1);
		expect(fineDragMultiplier(23)).toBe(1);
	});

	test("steps up as the pointer moves away, in either direction", () => {
		expect(fineDragMultiplier(24)).toBe(10);
		expect(fineDragMultiplier(-95)).toBe(10);
		expect(fineDragMultiplier(96)).toBe(100);
		expect(fineDragMultiplier(-400)).toBe(100);
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

		const moved = advanceFineDrag(drag, { xPx: 600, dyPx: 40, msPerPx });

		expect(moved.valueMs).toBe(100_500);
	});

	test("re-anchors on the current value so precision changes do not jump", () => {
		let drag = beginFineDrag(500, 100_000);
		const coarse = advanceFineDrag(drag, { xPx: 600, dyPx: 0, msPerPx });
		drag = coarse.state;

		const atThreshold = advanceFineDrag(drag, { xPx: 600, dyPx: 40, msPerPx });

		expect(atThreshold.valueMs).toBe(coarse.valueMs);
		expect(atThreshold.state.multiplier).toBe(10);

		const finer = advanceFineDrag(atThreshold.state, {
			xPx: 620,
			dyPx: 40,
			msPerPx,
		});

		expect(finer.valueMs).toBe(105_100);
	});

	test("returns to full rate when the pointer comes back", () => {
		let drag = beginFineDrag(500, 100_000);
		drag = advanceFineDrag(drag, { xPx: 600, dyPx: 200, msPerPx }).state;

		const back = advanceFineDrag(drag, { xPx: 600, dyPx: 0, msPerPx });

		expect(back.state.multiplier).toBe(1);
		expect(back.valueMs).toBe(100_050);
	});
});
