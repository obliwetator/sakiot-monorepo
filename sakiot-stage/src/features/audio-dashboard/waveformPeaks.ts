export function decodeWaveformPeaks(base64: string): number[] {
	const binary = atob(base64);
	const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
	if (bytes.byteLength < 20) return [];
	const view = new DataView(bytes.buffer);
	const flags = view.getUint32(4, true);
	const length = view.getUint32(16, true);
	const sixteenBit = flags === 1;
	const peaks: number[] = [];
	let offset = 20;
	for (let index = 0; index < length; index += 1) {
		if (sixteenBit) {
			if (offset + 2 > view.byteLength) break;
			peaks.push(view.getInt16(offset, true) / 32768);
			offset += 2;
		} else {
			if (offset + 1 > view.byteLength) break;
			peaks.push(view.getInt8(offset) / 128);
			offset += 1;
		}
	}
	return peaks;
}
