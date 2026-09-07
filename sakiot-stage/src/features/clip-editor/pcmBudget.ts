import { type ClipEdit, isTrackMuted, type TimelineSegment } from "./model";

/** Bounds retained cache ownership. Playback/UI references and in-flight work
 * remain separately owned and are not made collectible by cache eviction. */
export class PcmBudget<K> {
	private readonly entries = new Map<K, { bytes: number; evict: () => void }>();
	private bytes = 0;
	constructor(readonly limit: number) {}

	touch(key: K) {
		const entry = this.entries.get(key);
		if (!entry) return;
		this.entries.delete(key);
		this.entries.set(key, entry);
	}

	delete(key: K) {
		const entry = this.entries.get(key);
		if (!entry) return;
		this.bytes -= entry.bytes;
		this.entries.delete(key);
	}

	retain(key: K, bytes: number, evict: () => void) {
		this.delete(key);
		if (bytes > this.limit) {
			evict();
			return;
		}
		this.entries.set(key, { bytes, evict });
		this.bytes += bytes;
		while (this.bytes > this.limit) {
			const oldest = this.entries.entries().next().value;
			if (!oldest) break;
			this.delete(oldest[0]);
			oldest[1].evict();
		}
	}
}

export const SOURCE_CACHE_BYTES = 64 * 1024 * 1024;
export const PROCESSED_CACHE_BYTES = 64 * 1024 * 1024;
// Web Audio playback and waveform consumers still require a complete result.
export const MAX_BROWSER_RENDER_BYTES = 64 * 1024 * 1024;

export function browserSegmentBytes(
	segment: TimelineSegment,
	sampleRate = 48_000,
) {
	const sourceFrames = Math.max(
		0,
		Math.round(segment.sourceOut * sampleRate) -
			Math.round(segment.sourceIn * sampleRate),
	);
	const outputFrames =
		Math.round(sourceFrames / Math.fround(segment.effects.rate)) +
		Math.round(Math.fround(segment.effects.tailSeconds) * sampleRate);
	return { source: sourceFrames * 8, output: outputFrames * 8 };
}

export function browserPreviewLimited(edit: ClipEdit): boolean {
	let total = 0;
	return edit.segments.some((segment) => {
		if (isTrackMuted(edit, segment.track)) return false;
		const bytes = browserSegmentBytes(segment);
		total += bytes.output;
		return (
			bytes.source > MAX_BROWSER_RENDER_BYTES ||
			bytes.output > MAX_BROWSER_RENDER_BYTES ||
			total > 2 * MAX_BROWSER_RENDER_BYTES
		);
	});
}

export const BROWSER_PREVIEW_LIMIT_MESSAGE =
	"This edit is too large for browser preview. Shorten segments or preview fewer tracks. Server export remains available.";
