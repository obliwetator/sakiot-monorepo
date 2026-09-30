import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { browserStorage, optionalStorage } from "./browserStorage";
import { loadDraft, saveDraft } from "./draftStorage";
import {
	DEFAULT_EDITOR_OPTIONS,
	loadEditorOptions,
	saveEditorOptions,
} from "./editorOptions";
import { loadEffectLimits, saveEffectLimits } from "./effectLimits";
import { emptyEdit } from "./model";

/**
 * With site data blocked, merely reading `localStorage` throws. The editor
 * loads its options and limits during its first render, so an unguarded read
 * took the whole page down instead of only disabling persistence.
 */
describe("blocked browser storage", () => {
	let original: PropertyDescriptor | undefined;

	beforeEach(() => {
		original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
		Object.defineProperty(globalThis, "localStorage", {
			configurable: true,
			get() {
				throw new DOMException("Access is denied", "SecurityError");
			},
		});
	});

	afterEach(() => {
		if (original) Object.defineProperty(globalThis, "localStorage", original);
		else Reflect.deleteProperty(globalThis, "localStorage");
	});

	test("reports the failure instead of throwing", () => {
		const access = browserStorage();
		expect(access.ok).toBe(false);
		expect(optionalStorage()).toBeNull();
	});

	test("preferences fall back to defaults and skip saving", () => {
		expect(loadEditorOptions()).toEqual(DEFAULT_EDITOR_OPTIONS);
		expect(() => saveEditorOptions(DEFAULT_EDITOR_OPTIONS)).not.toThrow();
		const limits = loadEffectLimits();
		expect(() => saveEffectLimits(limits)).not.toThrow();
	});

	test("drafts report storage as unavailable", () => {
		const identity = { userId: "u", guildId: "g", sourceClipId: null };
		expect(loadDraft(identity)).toEqual({ status: "unavailable" });
		expect(
			saveDraft(identity, emptyEdit(), {
				expectedRaw: null,
				writer: "tab",
				savedAt: 0,
			}),
		).toEqual({ status: "error", reason: "unavailable" });
	});
});
