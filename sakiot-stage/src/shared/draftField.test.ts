import { describe, expect, test } from "bun:test";
import { type DraftState, draftReducer, viewDraft } from "./draftField";

const same = (a: number, b: number) => a === b;
const reduce = (
	state: DraftState<number> | null,
	action: Parameters<typeof draftReducer<number>>[1],
) => draftReducer(state, action, same);

describe("draft fields", () => {
	test("an untouched field follows the saved value", () => {
		expect(viewDraft(null, 30, "g1", same)).toEqual({
			value: 30,
			dirty: false,
			savedUpdate: null,
			settled: false,
		});
		expect(viewDraft(null, 45, "g1", same).value).toBe(45);
	});

	test("an edited field keeps its draft through refreshes", () => {
		const draft = reduce(null, {
			type: "edit",
			value: 60,
			scope: "g1",
			saved: 30,
		});
		// The same saved value refreshing again is not a change.
		expect(viewDraft(draft, 30, "g1", same)).toEqual({
			value: 60,
			dirty: true,
			savedUpdate: null,
			settled: false,
		});
		// Another admin saved 90: the draft stays and the change is surfaced.
		expect(viewDraft(draft, 90, "g1", same)).toEqual({
			value: 60,
			dirty: true,
			savedUpdate: { value: 90 },
			settled: false,
		});
	});

	test("the user resolves a change underneath their draft", () => {
		const draft = reduce(null, {
			type: "edit",
			value: 60,
			scope: "g1",
			saved: 30,
		});
		// Use saved value: the field follows the saved value again.
		expect(
			viewDraft(reduce(draft, { type: "use-saved" }), 90, "g1", same),
		).toMatchObject({ value: 90, dirty: false });
	});

	test("a saved draft holds until the saved value catches up", () => {
		const draft = reduce(null, {
			type: "edit",
			value: 60,
			scope: "g1",
			saved: 30,
		});
		const saved = reduce(draft, { type: "saved" });
		// The refetch is still in flight: no flash of the old value.
		expect(viewDraft(saved, 30, "g1", same)).toEqual({
			value: 60,
			dirty: false,
			savedUpdate: null,
			settled: false,
		});
		expect(viewDraft(saved, 60, "g1", same).settled).toBe(true);
		expect(reduce(saved, { type: "settled" })).toBeNull();
	});

	test("editing back to the saved value is no draft at all", () => {
		const draft = reduce(null, {
			type: "edit",
			value: 60,
			scope: "g1",
			saved: 30,
		});
		expect(
			reduce(draft, { type: "edit", value: 30, scope: "g1", saved: 30 }),
		).toBeNull();
	});

	test("another guild never shows this guild's draft", () => {
		const draft = reduce(null, {
			type: "edit",
			value: 60,
			scope: "g1",
			saved: 30,
		});
		expect(viewDraft(draft, 10, "g2", same)).toMatchObject({
			value: 10,
			dirty: false,
		});
	});
});
