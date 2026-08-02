import { describe, expect, test } from "bun:test";
import { isResetClipSelectionShortcut } from "./clipSelectionShortcuts";

const keyState = {
	key: "r",
	repeat: false,
	ctrlKey: false,
	metaKey: false,
	altKey: false,
};

describe("isResetClipSelectionShortcut", () => {
	test("accepts a plain lower- or upper-case R", () => {
		expect(isResetClipSelectionShortcut(keyState)).toBe(true);
		expect(isResetClipSelectionShortcut({ ...keyState, key: "R" })).toBe(true);
	});

	test("leaves modified and repeated shortcuts alone", () => {
		expect(isResetClipSelectionShortcut({ ...keyState, ctrlKey: true })).toBe(
			false,
		);
		expect(isResetClipSelectionShortcut({ ...keyState, metaKey: true })).toBe(
			false,
		);
		expect(isResetClipSelectionShortcut({ ...keyState, altKey: true })).toBe(
			false,
		);
		expect(isResetClipSelectionShortcut({ ...keyState, repeat: true })).toBe(
			false,
		);
	});
});
