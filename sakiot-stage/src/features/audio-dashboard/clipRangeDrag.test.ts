import { describe, expect, test } from "bun:test";
import {
	computeSelectionDrag,
	type SelectionDragState,
	selectionDragFeedback,
} from "./clipRangeDrag";
import { beginFineDrag } from "./clipSelection";

function edgeDrag(): SelectionDragState {
	return {
		kind: { type: "edge", edge: "start" },
		startX: 500,
		startY: 500,
		plotLeftPx: 100,
		plotWidthPx: 1_200,
		fine: beginFineDrag(500, 100_000),
		origin: [100_000, 130_000],
		movementWindow: { startMs: 0, endMs: 200_000 },
		moved: false,
		selection: [100_000, 130_000],
		valid: true,
	};
}

describe("computeSelectionDrag", () => {
	test("coordinates normal, fine, and ultra-fine edge movement", () => {
		const normal = computeSelectionDrag(
			edgeDrag(),
			{ clientX: 520, clientY: 500 },
			50,
			300_000,
			600,
		);
		expect(normal.selection[0]).toBe(101_000);
		expect(normal.fine.multiplier).toBe(1);

		const enteredFine = computeSelectionDrag(
			normal,
			{ clientX: 520, clientY: 460 },
			50,
			300_000,
			600,
		);
		expect(enteredFine.selection[0]).toBe(101_000);
		expect(enteredFine.fine.multiplier).toBe(10);

		const fine = computeSelectionDrag(
			enteredFine,
			{ clientX: 620, clientY: 460 },
			50,
			300_000,
			600,
		);
		expect(fine.selection[0]).toBe(101_500);

		const enteredUltra = computeSelectionDrag(
			fine,
			{ clientX: 620, clientY: 330 },
			50,
			300_000,
			600,
		);
		expect(enteredUltra.selection[0]).toBe(101_500);
		expect(enteredUltra.fine.multiplier).toBe(100);
	});

	test("clamps band at viewport boundary", () => {
		const drag: SelectionDragState = {
			...edgeDrag(),
			kind: { type: "band" },
			startX: 100,
			fine: beginFineDrag(100, 10_000),
			origin: [10_000, 20_000],
			movementWindow: { startMs: 0, endMs: 30_000 },
			selection: [10_000, 20_000],
		};
		const result = computeSelectionDrag(
			drag,
			{ clientX: -20_000, clientY: 500 },
			1,
			30_000,
			600,
		);
		expect(result.selection).toEqual([0, 10_000]);
		expect(result.fine.valueMs).toBe(0);
	});

	test("marks release below plot tolerance invalid", () => {
		const result = computeSelectionDrag(
			edgeDrag(),
			{ clientX: 520, clientY: 633 },
			50,
			300_000,
			600,
		);
		expect(result.valid).toBe(false);
	});

	test("feedback mirrors pure transition geometry", () => {
		const drag = computeSelectionDrag(
			edgeDrag(),
			{ clientX: 540, clientY: 460 },
			50,
			300_000,
			600,
		);
		expect(
			selectionDragFeedback(drag, { clientX: 540, clientY: 460 }),
		).toMatchObject({
			kind: { type: "edge", edge: "start" },
			multiplier: 10,
			dyPx: -40,
			pointerXPx: 540,
			pointerYPx: 460,
		});
	});
});
