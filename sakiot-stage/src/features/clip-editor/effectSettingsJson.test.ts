import { describe, expect, test } from "bun:test";
import {
	isEffectSettingsJsonShortcut,
	parseEffectSettingsJson,
} from "./effectSettingsJson";

describe("parseEffectSettingsJson", () => {
	test("accepts a partial effect patch", () => {
		expect(
			parseEffectSettingsJson('{"tailSeconds":2,"reverbWet":0.3}'),
		).toEqual({
			ok: true,
			patch: { tailSeconds: 2, reverbWet: 0.3 },
		});
	});

	test("unwraps a Markdown JSON fence", () => {
		expect(parseEffectSettingsJson('```json\n{"reverse":true}\n```')).toEqual({
			ok: true,
			patch: { reverse: true },
		});
	});

	test("rejects malformed, empty, and non-object JSON", () => {
		expect(parseEffectSettingsJson("{").ok).toBe(false);
		expect(parseEffectSettingsJson("{}")).toEqual({
			ok: false,
			error: "Include at least one effect setting.",
		});
		expect(parseEffectSettingsJson("[]").ok).toBe(false);
	});

	test("rejects unknown, mistyped, and out-of-range values", () => {
		expect(parseEffectSettingsJson('{"delayWte":0.2}')).toEqual({
			ok: false,
			error: "Unknown effect setting “delayWte”.",
		});
		expect(parseEffectSettingsJson('{"reverse":1}')).toEqual({
			ok: false,
			error: "“reverse” must be a boolean.",
		});
		expect(parseEffectSettingsJson('{"tailSeconds":31}')).toEqual({
			ok: false,
			error: "“tailSeconds” must be between 0 and 30.",
		});
		expect(parseEffectSettingsJson('{"reverbSeed":1.5}')).toEqual({
			ok: false,
			error: "“reverbSeed” must be an integer.",
		});
	});
});

describe("isEffectSettingsJsonShortcut", () => {
	const event = {
		altKey: false,
		ctrlKey: true,
		key: "O",
		metaKey: false,
		shiftKey: true,
	};

	test("recognizes Ctrl+Shift+O and Command+Shift+O", () => {
		expect(isEffectSettingsJsonShortcut(event)).toBe(true);
		expect(
			isEffectSettingsJsonShortcut({ ...event, ctrlKey: false, metaKey: true }),
		).toBe(true);
	});

	test("does not consume nearby shortcuts", () => {
		expect(isEffectSettingsJsonShortcut({ ...event, shiftKey: false })).toBe(
			false,
		);
		expect(isEffectSettingsJsonShortcut({ ...event, key: "P" })).toBe(false);
		expect(isEffectSettingsJsonShortcut({ ...event, altKey: true })).toBe(
			false,
		);
	});
});
