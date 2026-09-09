import { describe, expect, test } from "bun:test";
import { DEFAULT_EFFECT_LIMITS } from "./effectLimits";
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

describe("parseEffectSettingsJson with active limits", () => {
	const widened = {
		...DEFAULT_EFFECT_LIMITS,
		volumeDb: [-240, 240] as const,
	};

	test("accepts values the user widened past the hardcoded defaults", () => {
		// The hardcoded range is -80..24; the slider limits allow -240..240.
		expect(parseEffectSettingsJson('{"volumeDb":120}').ok).toBe(false);
		expect(parseEffectSettingsJson('{"volumeDb":120}', widened)).toEqual({
			ok: true,
			patch: { volumeDb: 120 },
		});
	});

	test("still rejects values outside the active limits", () => {
		const result = parseEffectSettingsJson('{"volumeDb":300}', widened);
		expect(result.ok).toBe(false);
		if (!result.ok) expect(result.error).toContain("between -240 and 240");
	});

	test("falls back to the safety caps for a corrupted limits object", () => {
		const corrupted = {
			...DEFAULT_EFFECT_LIMITS,
			pitchCents: [10, -10] as [number, number],
		};
		// An inverted pair falls back to the default range, not to "anything".
		expect(parseEffectSettingsJson('{"pitchCents":2400}', corrupted).ok).toBe(
			true,
		);
		expect(parseEffectSettingsJson('{"pitchCents":9999}', corrupted).ok).toBe(
			false,
		);
	});
});
