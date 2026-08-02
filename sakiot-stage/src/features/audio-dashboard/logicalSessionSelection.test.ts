import { describe, expect, it } from "bun:test";
import {
	defaultClipSelection,
	isValidClipSelection,
	reconcileSessionSelection,
	resetClipSelection,
	selectionAroundStamp,
} from "./logicalSessionSelection";

describe("reconcileSessionSelection", () => {
	it("starts a new recording with a valid short clip draft", () => {
		expect(
			reconcileSessionSelection(
				[0, 12_000],
				{ recordingSessionId: "first", durationMs: 12_000 },
				{ recordingSessionId: "second", durationMs: 45_000 },
			),
		).toEqual([0, 15_000]);
	});

	it("keeps max-pinned selection at max while recording grows", () => {
		expect(
			reconcileSessionSelection(
				[5_000, 12_000],
				{ recordingSessionId: "same", durationMs: 12_000 },
				{ recordingSessionId: "same", durationMs: 15_000 },
			),
		).toEqual([5_000, 15_000]);
	});

	it("preserves custom selection during manifest polling", () => {
		const selection: [number, number] = [2_000, 8_000];
		expect(
			reconcileSessionSelection(
				selection,
				{ recordingSessionId: "same", durationMs: 12_000 },
				{ recordingSessionId: "same", durationMs: 15_000 },
			),
		).toBe(selection);
	});

	it("stops growing the automatic draft once it reaches fifteen seconds", () => {
		expect(
			reconcileSessionSelection(
				[0, 15_000],
				{ recordingSessionId: "same", durationMs: 20_000 },
				{ recordingSessionId: "same", durationMs: 25_000 },
			),
		).toEqual([0, 15_000]);
	});
});

describe("defaultClipSelection", () => {
	it("selects at most fifteen seconds", () => {
		expect(defaultClipSelection(22_484_000)).toEqual([0, 15_000]);
		expect(defaultClipSelection(8_000)).toEqual([0, 8_000]);
	});
});

describe("isValidClipSelection", () => {
	it("accepts selections from one through twenty seconds", () => {
		expect(isValidClipSelection([0, 999])).toBe(false);
		expect(isValidClipSelection([5_000, 6_000])).toBe(true);
		expect(isValidClipSelection([5_000, 25_000])).toBe(true);
		expect(isValidClipSelection([5_000, 25_001])).toBe(false);
	});
});

describe("selectionAroundStamp", () => {
	it("puts most of the draft before the stamped moment", () => {
		expect(selectionAroundStamp(60_000, 300_000)).toEqual([50_000, 65_000]);
	});

	it("slides the complete draft inside the beginning and end", () => {
		expect(selectionAroundStamp(2_000, 300_000)).toEqual([0, 15_000]);
		expect(selectionAroundStamp(299_000, 300_000)).toEqual([285_000, 300_000]);
	});

	it("uses the complete session when it is shorter than a draft", () => {
		expect(selectionAroundStamp(2_000, 8_000)).toEqual([0, 8_000]);
	});
});

describe("resetClipSelection", () => {
	it("restores the normal session draft", () => {
		expect(resetClipSelection(300_000)).toEqual([0, 15_000]);
	});

	it("restores a stamp draft without discarding its position", () => {
		expect(resetClipSelection(300_000, 60_000)).toEqual([50_000, 65_000]);
	});
});
