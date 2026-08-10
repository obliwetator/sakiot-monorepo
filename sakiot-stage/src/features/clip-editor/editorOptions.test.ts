import { describe, expect, test } from "bun:test";
import {
	DEFAULT_EDITOR_OPTIONS,
	loadEditorOptions,
	type StorageLike,
	saveEditorOptions,
} from "./editorOptions";

function storage(initial: Record<string, string> = {}): StorageLike & {
	store: Record<string, string>;
} {
	const store = { ...initial };
	return {
		store,
		getItem(key: string) {
			return store[key] ?? null;
		},
		setItem(key: string, value: string) {
			store[key] = value;
		},
	};
}

describe("loadEditorOptions", () => {
	test("returns the defaults when nothing is stored", () => {
		expect(loadEditorOptions(storage())).toEqual(DEFAULT_EDITOR_OPTIONS);
		expect(loadEditorOptions(storage()).marqueeMultiTrack).toBe(false);
	});

	test("round-trips saved options", () => {
		const store = storage();
		saveEditorOptions({ marqueeMultiTrack: true }, store);
		expect(loadEditorOptions(store).marqueeMultiTrack).toBe(true);
	});

	test("falls back to defaults for corrupt storage", () => {
		const options = loadEditorOptions(
			storage({ "sakiot:clip-editor:options": "{not json" }),
		);
		expect(options).toEqual(DEFAULT_EDITOR_OPTIONS);
	});

	test("ignores unknown keys and non-boolean values", () => {
		expect(
			loadEditorOptions(
				storage({
					"sakiot:clip-editor:options": JSON.stringify({
						marqueeMultiTrack: "yes",
					}),
				}),
			),
		).toEqual(DEFAULT_EDITOR_OPTIONS);
		expect(
			loadEditorOptions(
				storage({
					"sakiot:clip-editor:options": JSON.stringify({
						someFutureOption: true,
					}),
				}),
			),
		).toEqual(DEFAULT_EDITOR_OPTIONS);
		// Unknown keys never leak into the returned options object.
		expect(
			Object.keys(
				loadEditorOptions(
					storage({
						"sakiot:clip-editor:options": JSON.stringify({
							marqueeMultiTrack: true,
							someFutureOption: 42,
						}),
					}),
				),
			),
		).toEqual(Object.keys(DEFAULT_EDITOR_OPTIONS));
	});
});
