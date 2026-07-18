import { describe, expect, it } from "bun:test";
import {
	isValidClipSelection,
	reconcileSessionSelection,
} from "./logicalSessionSelection";

describe("reconcileSessionSelection", () => {
	it("resets selection to new recording max when recordings change", () => {
		expect(
			reconcileSessionSelection(
				[0, 12_000],
				{ recordingSessionId: "first", durationMs: 12_000 },
				{ recordingSessionId: "second", durationMs: 45_000 },
			),
		).toEqual([0, 45_000]);
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
});

describe("isValidClipSelection", () => {
	it("accepts selections from one through twenty seconds", () => {
		expect(isValidClipSelection([0, 999])).toBe(false);
		expect(isValidClipSelection([5_000, 6_000])).toBe(true);
		expect(isValidClipSelection([5_000, 25_000])).toBe(true);
		expect(isValidClipSelection([5_000, 25_001])).toBe(false);
	});
});
