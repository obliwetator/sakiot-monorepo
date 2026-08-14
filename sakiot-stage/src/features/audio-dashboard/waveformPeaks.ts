/**
 * Minimum and maximum sample of every waveform point, normalised to -1..1.
 * Both arrays hold one entry per point and always share a length.
 */
export interface WaveformEnvelope {
	min: number[];
	max: number[];
	/** Duration inferred from the audiowaveform sample-rate metadata. */
	durationMs?: number;
}

export const EMPTY_WAVEFORM_ENVELOPE: WaveformEnvelope = { min: [], max: [] };

const HEADER_BYTES = 20;
const FLAG_EIGHT_BIT = 1;

/**
 * Decodes an audiowaveform .dat payload.
 *
 * Header layout (version 1): version, flags, sample rate, samples per pixel,
 * then `length` — the number of min/max *pairs*, so the data section holds
 * `2 * length` values. Flag 1 means 8-bit samples, 0 means 16-bit.
 */
export function decodeWaveformPeaks(base64: string): WaveformEnvelope {
	const binary = atob(base64);
	const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
	if (bytes.byteLength < HEADER_BYTES) return EMPTY_WAVEFORM_ENVELOPE;
	const view = new DataView(bytes.buffer);
	const eightBit = view.getUint32(4, true) === FLAG_EIGHT_BIT;
	const sampleRate = view.getUint32(8, true);
	const samplesPerPoint = view.getUint32(12, true);
	const pointCount = view.getUint32(16, true);
	const bytesPerValue = eightBit ? 1 : 2;
	const scale = eightBit ? 128 : 32768;

	const min: number[] = [];
	const max: number[] = [];
	let offset = HEADER_BYTES;
	for (let point = 0; point < pointCount; point += 1) {
		if (offset + bytesPerValue * 2 > view.byteLength) break;
		if (eightBit) {
			min.push(view.getInt8(offset) / scale);
			max.push(view.getInt8(offset + 1) / scale);
		} else {
			min.push(view.getInt16(offset, true) / scale);
			max.push(view.getInt16(offset + 2, true) / scale);
		}
		offset += bytesPerValue * 2;
	}
	const durationMs =
		sampleRate > 0 && samplesPerPoint > 0
			? (min.length * samplesPerPoint * 1_000) / sampleRate
			: undefined;
	return { min, max, durationMs };
}
