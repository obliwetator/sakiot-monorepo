import type { components } from "../../api/openapi";
import type { ClipEdit, SegmentEffects, TimelineSegment } from "./model";
import { DEFAULT_EFFECTS, newSegmentId } from "./model";

type ComposeClipBody = components["schemas"]["ComposeClipBody"];

/**
 * Mirrors the ClipEdit state onto the server's snake_case composition
 * request. The export endpoint renders exactly what the Web Audio engine
 * previews: every shared per-segment DSP parameter, mixed onto the timeline
 * with the master volume applied to the result.
 */
export function serializeEdit(edit: ClipEdit, name?: string): ComposeClipBody {
	return {
		name,
		master_volume_db: edit.masterVolumeDb,
		segments: edit.segments.map((segment) => ({
			source: segment.source,
			source_id: segment.sourceId,
			source_in: segment.sourceIn,
			source_out: segment.sourceOut,
			timeline_start: segment.timelineStart,
			track: segment.track,
			merge_group: segment.mergeGroup,
			effects: {
				volume_db: segment.effects.volumeDb,
				pitch_cents: segment.effects.pitchCents,
				rate: segment.effects.rate,
				bass_db: segment.effects.bassDb,
				mid_db: segment.effects.midDb,
				treble_db: segment.effects.trebleDb,
				advanced: {
					tail_seconds: segment.effects.tailSeconds,
					distortion_amount: segment.effects.distortionAmount,
					distortion_wet: segment.effects.distortionWet,
					delay_seconds: segment.effects.delaySeconds,
					delay_feedback: segment.effects.delayFeedback,
					delay_wet: segment.effects.delayWet,
					compressor_enabled: segment.effects.compressorEnabled,
					compressor_threshold_db: segment.effects.compressorThresholdDb,
					compressor_knee_db: segment.effects.compressorKneeDb,
					compressor_ratio: segment.effects.compressorRatio,
					compressor_attack_seconds: segment.effects.compressorAttackSeconds,
					compressor_release_seconds: segment.effects.compressorReleaseSeconds,
					chorus_enabled: segment.effects.chorusEnabled,
					chorus_frequency_hz: segment.effects.chorusFrequencyHz,
					chorus_delay_ms: segment.effects.chorusDelayMs,
					chorus_depth: segment.effects.chorusDepth,
					chorus_spread_degrees: segment.effects.chorusSpreadDegrees,
					chorus_feedback: segment.effects.chorusFeedback,
					chorus_wet: segment.effects.chorusWet,
					reverb_enabled: segment.effects.reverbEnabled,
					reverb_decay_seconds: segment.effects.reverbDecaySeconds,
					reverb_pre_delay_seconds: segment.effects.reverbPreDelaySeconds,
					reverb_wet: segment.effects.reverbWet,
					reverb_seed: segment.effects.reverbSeed,
				},
				reverse: segment.effects.reverse,
			},
		})),
	};
}

/**
 * Inverse of serializeEdit: rebuilds a ClipEdit from the composition JSON a
 * composed clip carries in the database. Returns null when the payload is
 * missing or malformed so callers can fall back to importing the clip as a
 * single segment.
 */
export function deserializeEdit(data: unknown): ClipEdit | null {
	if (typeof data !== "object" || data === null) return null;
	const record = data as Record<string, unknown>;
	const rawSegments = record.segments;
	if (!Array.isArray(rawSegments)) return null;
	const segments: TimelineSegment[] = [];
	for (const raw of rawSegments) {
		const segment = deserializeSegment(raw);
		if (!segment) return null;
		segments.push(segment);
	}
	const masterVolumeDb = record.master_volume_db;
	if (typeof masterVolumeDb !== "number" || !Number.isFinite(masterVolumeDb)) {
		return null;
	}
	const tracks = segments.reduce(
		(max, segment) => Math.max(max, segment.track + 1),
		1,
	);
	return { segments, tracks, masterVolumeDb };
}

function deserializeSegment(raw: unknown): TimelineSegment | null {
	if (typeof raw !== "object" || raw === null) return null;
	const segment = raw as Record<string, unknown>;
	if (
		segment.source !== "clip" &&
		segment.source !== "session" &&
		segment.source !== "session-silence-free"
	) {
		return null;
	}
	const {
		source_id: sourceId,
		source_in: sourceIn,
		source_out: sourceOut,
	} = segment;
	const timelineStart = segment.timeline_start;
	const track = segment.track;
	const mergeGroup = segment.merge_group;
	const effects = deserializeEffects(segment.effects);
	if (typeof sourceId !== "string") return null;
	if (!isFiniteNumber(sourceIn) || !isFiniteNumber(sourceOut)) return null;
	if (!isFiniteNumber(timelineStart) || timelineStart < 0) return null;
	if (typeof track !== "number" || !Number.isInteger(track) || track < 0) {
		return null;
	}
	if (mergeGroup !== undefined && mergeGroup !== null) {
		if (typeof mergeGroup !== "string" || mergeGroup.length === 0) return null;
	}
	if (!effects) return null;
	return {
		id: newSegmentId(),
		track,
		source: segment.source as TimelineSegment["source"],
		sourceId,
		sourceIn,
		sourceOut,
		timelineStart,
		effects,
		...(typeof mergeGroup === "string" ? { mergeGroup } : {}),
	};
}

function deserializeEffects(raw: unknown): SegmentEffects | null {
	if (typeof raw !== "object" || raw === null) return null;
	const effects = raw as Record<string, unknown>;
	const { volume_db: volumeDb, pitch_cents: pitchCents } = effects;
	const {
		rate,
		bass_db: bassDb,
		mid_db: midDb = 0,
		treble_db: trebleDb,
	} = effects;
	if (
		effects.advanced !== undefined &&
		(typeof effects.advanced !== "object" || effects.advanced === null)
	)
		return null;
	const advanced = (effects.advanced ?? {}) as Record<string, unknown>;
	const tailSeconds = advanced.tail_seconds ?? DEFAULT_EFFECTS.tailSeconds;
	const distortionAmount =
		advanced.distortion_amount ?? DEFAULT_EFFECTS.distortionAmount;
	const distortionWet =
		advanced.distortion_wet ?? DEFAULT_EFFECTS.distortionWet;
	const delaySeconds = advanced.delay_seconds ?? DEFAULT_EFFECTS.delaySeconds;
	const delayFeedback =
		advanced.delay_feedback ?? DEFAULT_EFFECTS.delayFeedback;
	const delayWet = advanced.delay_wet ?? DEFAULT_EFFECTS.delayWet;
	const compressorEnabled =
		advanced.compressor_enabled ?? DEFAULT_EFFECTS.compressorEnabled;
	const compressorThresholdDb =
		advanced.compressor_threshold_db ?? DEFAULT_EFFECTS.compressorThresholdDb;
	const compressorKneeDb =
		advanced.compressor_knee_db ?? DEFAULT_EFFECTS.compressorKneeDb;
	const compressorRatio =
		advanced.compressor_ratio ?? DEFAULT_EFFECTS.compressorRatio;
	const compressorAttackSeconds =
		advanced.compressor_attack_seconds ??
		DEFAULT_EFFECTS.compressorAttackSeconds;
	const compressorReleaseSeconds =
		advanced.compressor_release_seconds ??
		DEFAULT_EFFECTS.compressorReleaseSeconds;
	const chorusEnabled =
		advanced.chorus_enabled ?? DEFAULT_EFFECTS.chorusEnabled;
	const chorusFrequencyHz =
		advanced.chorus_frequency_hz ?? DEFAULT_EFFECTS.chorusFrequencyHz;
	const chorusDelayMs =
		advanced.chorus_delay_ms ?? DEFAULT_EFFECTS.chorusDelayMs;
	const chorusDepth = advanced.chorus_depth ?? DEFAULT_EFFECTS.chorusDepth;
	const chorusSpreadDegrees =
		advanced.chorus_spread_degrees ?? DEFAULT_EFFECTS.chorusSpreadDegrees;
	const chorusFeedback =
		advanced.chorus_feedback ?? DEFAULT_EFFECTS.chorusFeedback;
	const chorusWet = advanced.chorus_wet ?? DEFAULT_EFFECTS.chorusWet;
	const reverbEnabled =
		advanced.reverb_enabled ?? DEFAULT_EFFECTS.reverbEnabled;
	const reverbDecaySeconds =
		advanced.reverb_decay_seconds ?? DEFAULT_EFFECTS.reverbDecaySeconds;
	const reverbPreDelaySeconds =
		advanced.reverb_pre_delay_seconds ?? DEFAULT_EFFECTS.reverbPreDelaySeconds;
	const reverbWet = advanced.reverb_wet ?? DEFAULT_EFFECTS.reverbWet;
	const reverbSeed = advanced.reverb_seed ?? DEFAULT_EFFECTS.reverbSeed;
	const reverse = effects.reverse;
	if (
		!isFiniteNumber(volumeDb) ||
		!isFiniteNumber(pitchCents) ||
		!isFiniteNumber(rate) ||
		!isFiniteNumber(bassDb) ||
		!isFiniteNumber(midDb) ||
		!isFiniteNumber(trebleDb) ||
		!isFiniteNumber(tailSeconds) ||
		!isFiniteNumber(distortionAmount) ||
		!isFiniteNumber(distortionWet) ||
		!isFiniteNumber(delaySeconds) ||
		!isFiniteNumber(delayFeedback) ||
		!isFiniteNumber(delayWet) ||
		!isFiniteNumber(compressorThresholdDb) ||
		!isFiniteNumber(compressorKneeDb) ||
		!isFiniteNumber(compressorRatio) ||
		!isFiniteNumber(compressorAttackSeconds) ||
		!isFiniteNumber(compressorReleaseSeconds) ||
		!isFiniteNumber(chorusFrequencyHz) ||
		!isFiniteNumber(chorusDelayMs) ||
		!isFiniteNumber(chorusDepth) ||
		!isFiniteNumber(chorusSpreadDegrees) ||
		!isFiniteNumber(chorusFeedback) ||
		!isFiniteNumber(chorusWet) ||
		!isFiniteNumber(reverbDecaySeconds) ||
		!isFiniteNumber(reverbPreDelaySeconds) ||
		!isFiniteNumber(reverbWet) ||
		!isUint32(reverbSeed)
	) {
		return null;
	}
	if (
		typeof compressorEnabled !== "boolean" ||
		typeof chorusEnabled !== "boolean" ||
		typeof reverbEnabled !== "boolean"
	)
		return null;
	// Compositions exported before reverse existed omit the flag; treat the
	// absence as forward playback.
	if (reverse !== undefined && typeof reverse !== "boolean") return null;
	return {
		volumeDb,
		pitchCents,
		rate,
		bassDb,
		midDb,
		trebleDb,
		tailSeconds,
		distortionAmount,
		distortionWet,
		delaySeconds,
		delayFeedback,
		delayWet,
		compressorEnabled,
		compressorThresholdDb,
		compressorKneeDb,
		compressorRatio,
		compressorAttackSeconds,
		compressorReleaseSeconds,
		chorusEnabled,
		chorusFrequencyHz,
		chorusDelayMs,
		chorusDepth,
		chorusSpreadDegrees,
		chorusFeedback,
		chorusWet,
		reverbEnabled,
		reverbDecaySeconds,
		reverbPreDelaySeconds,
		reverbWet,
		reverbSeed,
		reverse: reverse === true,
	};
}

function isFiniteNumber(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value);
}

function isUint32(value: unknown): value is number {
	return (
		isFiniteNumber(value) &&
		Number.isInteger(value) &&
		value >= 0 &&
		value <= 0xffff_ffff
	);
}
