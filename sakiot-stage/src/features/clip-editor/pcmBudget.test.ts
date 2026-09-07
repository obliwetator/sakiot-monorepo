import { describe, expect, test } from "bun:test";
import { type ClipEdit, DEFAULT_EFFECTS } from "./model";
import { browserPreviewLimited, PcmBudget } from "./pcmBudget";

describe("PCM memory ownership", () => {
	test("evicts the least recently used value and refuses to retain oversized values", () => {
		const budget = new PcmBudget<string>(100);
		const evicted: string[] = [];
		budget.retain("a", 40, () => evicted.push("a"));
		budget.retain("b", 40, () => evicted.push("b"));
		budget.touch("a");
		budget.retain("c", 40, () => evicted.push("c"));
		expect(evicted).toEqual(["b"]);
		budget.delete("a");
		budget.retain("c", 90, () => evicted.push("c-new"));
		expect(evicted).toEqual(["b"]);
		budget.retain("huge", 101, () => evicted.push("huge"));
		expect(evicted).toEqual(["b", "huge"]);
	});

	test("preview limits account for source length, rate, tails, aggregate playback and muted tracks", () => {
		const segment = {
			id: "a",
			source: "clip" as const,
			sourceId: "clip",
			sourceIn: 0,
			sourceOut: 60,
			timelineStart: 0,
			track: 0,
			effects: { ...DEFAULT_EFFECTS },
		};
		const edit: ClipEdit = {
			segments: [segment],
			masterVolumeDb: 0,
			mutedTracks: [],
			tracks: 1,
		};
		expect(browserPreviewLimited(edit)).toBe(false);
		expect(
			browserPreviewLimited({
				...edit,
				segments: [
					{
						...segment,
						sourceOut: 160,
						effects: { ...segment.effects, tailSeconds: 20 },
					},
				],
			}),
		).toBe(true);
		const boundary = (64 * 1024 * 1024) / 8 / 48000;
		expect(
			browserPreviewLimited({
				...edit,
				segments: [{ ...segment, sourceOut: boundary }],
			}),
		).toBe(false);
		expect(
			browserPreviewLimited({
				...edit,
				segments: [{ ...segment, sourceOut: boundary + 1 / 48000 }],
			}),
		).toBe(true);
		expect(
			browserPreviewLimited({
				...edit,
				segments: [{ ...segment, sourceOut: 180 }],
			}),
		).toBe(true);
		expect(
			browserPreviewLimited({
				...edit,
				segments: [{ ...segment, effects: { ...segment.effects, rate: 0.1 } }],
			}),
		).toBe(true);
		expect(
			browserPreviewLimited({
				...edit,
				segments: Array.from({ length: 6 }, (_, i) => ({
					...segment,
					id: String(i),
				})),
			}),
		).toBe(true);
		expect(
			browserPreviewLimited({
				...edit,
				mutedTracks: [true],
				segments: [{ ...segment, sourceOut: 180 }],
			}),
		).toBe(false);
	});
});
