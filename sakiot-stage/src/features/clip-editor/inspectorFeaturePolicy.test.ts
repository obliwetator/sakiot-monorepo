import { describe, expect, test } from "bun:test";
import {
	isInspectorFeatureDisabled,
	isMultiSelectionDisabled,
} from "./inspectorFeaturePolicy";

describe("inspector multi-selection policy", () => {
	test("an undeclared policy is disabled for a multi-selection", () => {
		expect(isMultiSelectionDisabled(2)).toBe(true);
	});

	test("the gate never affects a single selection", () => {
		expect(isMultiSelectionDisabled(1)).toBe(false);
		expect(isInspectorFeatureDisabled("split", 1)).toBe(false);
	});

	test("an explicitly handled feature stays enabled for a multi-selection", () => {
		expect(isMultiSelectionDisabled(2, { multiSelection: "handled" })).toBe(
			false,
		);
		expect(isInspectorFeatureDisabled("pitch", 2)).toBe(false);
		expect(isInspectorFeatureDisabled("reverse", 3)).toBe(false);
	});

	test("an unhandled registered feature is disabled for a multi-selection", () => {
		expect(isInspectorFeatureDisabled("split", 2)).toBe(true);
	});
});
