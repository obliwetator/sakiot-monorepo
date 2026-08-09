/**
 * Parameters shared by the canonical DSP and the emergency native Web Audio
 * fallback. Normal preview and server rendering both use sakiot-DSP; this
 * graph is only used if WASM initialization fails.
 */
export const PARITY_APPROVED_EQ = {
	bass: {
		frequencyHz: 250,
		webAudioType: "lowshelf",
		width: 1,
	},
	mid: {
		frequencyHz: 1_000,
		webAudioType: "peaking",
		width: 1,
	},
	treble: {
		frequencyHz: 3_000,
		webAudioType: "highshelf",
		width: 1,
	},
} as const;

export type ParityApprovedEqId = keyof typeof PARITY_APPROVED_EQ;
