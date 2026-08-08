import { describe, expect, test } from "bun:test";
import {
	createHistoryState,
	HISTORY_LIMIT,
	type HistoryState,
	historyApply,
	historyFlush,
	historyPreview,
	historyRedo,
	historyUndo,
} from "./history";

type Edit = number[];

function addId(state: HistoryState<Edit>, id: number): HistoryState<Edit> {
	return historyApply(state, (edit) => [...edit, id]);
}

function ids(state: HistoryState<Edit>): Edit {
	return state.present;
}

describe("history", () => {
	test("apply pushes the previous present onto past and clears future", () => {
		let state = createHistoryState<Edit>([]);
		state = addId(state, 1);
		state = addId(state, 2);
		expect(ids(state)).toEqual([1, 2]);
		expect(state.past).toEqual([[], [1]]);
		expect(state.base).toEqual([1, 2]);
		expect(state.future).toEqual([]);
	});

	test("undo walks back and redo walks forward", () => {
		let state = createHistoryState<Edit>([]);
		state = addId(state, 1);
		state = addId(state, 2);
		state = historyUndo(state);
		expect(ids(state)).toEqual([1]);
		expect(state.past).toEqual([[]]);
		expect(state.future).toEqual([[1, 2]]);
		state = historyUndo(state);
		expect(ids(state)).toEqual([]);
		expect(state.past).toEqual([]);
		state = historyUndo(state);
		expect(ids(state)).toEqual([]);
		state = historyRedo(state);
		expect(ids(state)).toEqual([1]);
		state = historyRedo(state);
		expect(ids(state)).toEqual([1, 2]);
		state = historyRedo(state);
		expect(ids(state)).toEqual([1, 2]);
	});

	test("undo is available only while past is non-empty", () => {
		const state = createHistoryState<Edit>([]);
		expect(historyUndo(state)).toBe(state);
		expect(state.future).toEqual([]);
	});

	test("preview mutates present without touching history", () => {
		let state = createHistoryState<Edit>([]);
		state = historyPreview(state, (edit) => [...edit, 9]);
		expect(ids(state)).toEqual([9]);
		expect(state.past).toEqual([]);
		expect(state.future).toEqual([]);
		expect(state.base).toEqual([]);
	});

	test("flush folds every preview since the last flush into one step", () => {
		let state = createHistoryState<Edit>([]);
		state = historyPreview(state, (edit) => [...edit, 1]);
		state = historyPreview(state, (edit) => [...edit, 2]);
		state = historyPreview(state, (edit) => [...edit, 3]);
		state = historyFlush(state);
		expect(ids(state)).toEqual([1, 2, 3]);
		expect(state.past).toEqual([[]]);
		expect(state.base).toEqual([1, 2, 3]);
		const undone = historyUndo(state);
		expect(ids(undone)).toEqual([]);
		const redone = historyRedo(undone);
		expect(ids(redone)).toEqual([1, 2, 3]);
	});

	test("flush without previews is a no-op", () => {
		let state = createHistoryState<Edit>([]);
		state = addId(state, 1);
		expect(historyFlush(state)).toBe(state);
	});

	test("applying after a gesture keeps the gesture as one step", () => {
		let state = createHistoryState<Edit>([]);
		state = historyPreview(state, (edit) => [...edit, 1]);
		state = historyFlush(state);
		state = addId(state, 2);
		expect(state.past).toEqual([[], [1]]);
	});

	test("apply clears the redo stack", () => {
		let state = createHistoryState<Edit>([]);
		state = addId(state, 1);
		state = addId(state, 2);
		state = historyUndo(state);
		expect(state.future.length).toBe(1);
		state = addId(state, 3);
		expect(state.future).toEqual([]);
		expect(historyRedo(state)).toBe(state);
	});

	test("no-op updaters return the same state reference", () => {
		const state = createHistoryState<Edit>([1]);
		expect(historyApply(state, (edit) => edit)).toBe(state);
		expect(historyPreview(state, (edit) => edit)).toBe(state);
	});

	test("history is capped at HISTORY_LIMIT steps", () => {
		let state = createHistoryState<Edit>([]);
		for (let i = 0; i < HISTORY_LIMIT + 10; i += 1) {
			state = addId(state, i);
		}
		expect(state.past.length).toBe(HISTORY_LIMIT);
		expect(state.past[0]).toEqual(Array.from({ length: 10 }, (_, i) => i));
	});

	test("reset clears history", () => {
		let state = createHistoryState<Edit>([]);
		state = addId(state, 1);
		state = addId(state, 2);
		state = createHistoryState<Edit>([]);
		expect(ids(state)).toEqual([]);
		expect(state.past).toEqual([]);
		expect(state.future).toEqual([]);
		expect(historyUndo(state)).toBe(state);
	});
});
