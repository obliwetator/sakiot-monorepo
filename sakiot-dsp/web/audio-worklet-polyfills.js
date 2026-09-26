// AudioWorkletGlobalScope deliberately exposes fewer Web Platform APIs than a
// Window. wasm-bindgen uses TextDecoder both for diagnostics and for property
// names passed to the versioned JavaScript effect-config reader, so this must
// decode real UTF-8 rather than return a diagnostic placeholder.
globalThis.TextDecoder ??= class WorkletTextDecoder {
	decode(input = new Uint8Array()) {
		const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
		let output = "";
		for (let index = 0; index < bytes.length; ) {
			const first = bytes[index++] ?? 0;
			if (first < 0x80) {
				output += String.fromCharCode(first);
				continue;
			}
			const second = bytes[index++] ?? 0;
			if (first < 0xe0) {
				output += String.fromCharCode(((first & 0x1f) << 6) | (second & 0x3f));
				continue;
			}
			const third = bytes[index++] ?? 0;
			if (first < 0xf0) {
				output += String.fromCharCode(
					((first & 0x0f) << 12) | ((second & 0x3f) << 6) | (third & 0x3f),
				);
				continue;
			}
			const fourth = bytes[index++] ?? 0;
			const point =
				((first & 0x07) << 18) |
				((second & 0x3f) << 12) |
				((third & 0x3f) << 6) |
				(fourth & 0x3f);
			const adjusted = point - 0x1_0000;
			output += String.fromCharCode(
				0xd800 | (adjusted >> 10),
				0xdc00 | (adjusted & 0x3ff),
			);
		}
		return output;
	}
};
