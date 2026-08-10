import { describe, expect, test } from "bun:test";
import {
	clampLimitToSafety,
	DEFAULT_EFFECT_LIMITS,
	EFFECT_LIMIT_SAFETY_CAPS,
	loadEffectLimits,
	type StorageLike,
	saveEffectLimits,
	serializeLimits,
} from "./effectLimits";

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

describe("loadEffectLimits", () => {
	test("returns the doubled defaults when nothing is stored", () => {
		const limits = loadEffectLimits(storage());
		expect(limits).toEqual(DEFAULT_EFFECT_LIMITS);
		expect(limits.volumeDb).toEqual([-80, 24]);
		expect(limits.pitchCents).toEqual([-2400, 2400]);
		expect(limits.rate).toEqual([0.25, 4]);
		expect(limits.bassDb).toEqual([-24, 24]);
		expect(limits.midDb).toEqual([-24, 24]);
		expect(limits.trebleDb).toEqual([-24, 24]);
	});

	test("round-trips saved limits", () => {
		const store = storage();
		const custom = {
			...DEFAULT_EFFECT_LIMITS,
			volumeDb: [-100, 60],
			rate: [0.1, 8],
		};
		saveEffectLimits(custom, store);
		expect(loadEffectLimits(store)).toEqual(custom);
	});

	test("clamps stored limits to the safety caps", () => {
		const store = storage({
			"sakiot:clip-editor:effect-limits": JSON.stringify({
				volumeDb: [-1000, 1000],
				rate: [0.001, 100],
			}),
		});
		const limits = loadEffectLimits(store);
		expect(limits.volumeDb).toEqual([
			EFFECT_LIMIT_SAFETY_CAPS.volumeDb[0],
			EFFECT_LIMIT_SAFETY_CAPS.volumeDb[1],
		]);
		expect(limits.rate).toEqual([
			EFFECT_LIMIT_SAFETY_CAPS.rate[0],
			EFFECT_LIMIT_SAFETY_CAPS.rate[1],
		]);
		expect(limits.pitchCents).toEqual(DEFAULT_EFFECT_LIMITS.pitchCents);
	});

	test("falls back to defaults for corrupted or invalid ranges", () => {
		for (const raw of [
			"{not json",
			JSON.stringify({ volumeDb: [5, 5] }),
			JSON.stringify({ volumeDb: [1, 2, 3] }),
			JSON.stringify({ volumeDb: ["a", "b"] }),
		]) {
			const limits = loadEffectLimits(
				storage({ "sakiot:clip-editor:effect-limits": raw }),
			);
			expect(limits).toEqual(DEFAULT_EFFECT_LIMITS);
		}
	});
});

describe("clampLimitToSafety", () => {
	test("clamps out-of-cap ranges inward", () => {
		expect(clampLimitToSafety("volumeDb", [-300, 300])).toEqual([-240, 240]);
		expect(clampLimitToSafety("pitchCents", [-6000, 6000])).toEqual([
			-4800, 4800,
		]);
		expect(clampLimitToSafety("rate", [0.01, 20])).toEqual([0.1, 10]);
	});

	test("keeps in-cap ranges unchanged", () => {
		expect(clampLimitToSafety("bassDb", [-24, 24])).toEqual([-24, 24]);
	});

	test("falls back to defaults for inverted ranges", () => {
		expect(clampLimitToSafety("midDb", [10, -10])).toEqual(
			DEFAULT_EFFECT_LIMITS.midDb,
		);
	});
});

describe("serializeLimits", () => {
	test("maps camelCase limits to the snake_case request shape", () => {
		expect(serializeLimits(DEFAULT_EFFECT_LIMITS)).toEqual({
			volume_db_min: -80,
			volume_db_max: 24,
			pitch_cents_min: -2400,
			pitch_cents_max: 2400,
			rate_min: 0.25,
			rate_max: 4,
			bass_db_min: -24,
			bass_db_max: 24,
			mid_db_min: -24,
			mid_db_max: 24,
			treble_db_min: -24,
			treble_db_max: 24,
		});
	});
});
