/**
 * Effects admitted here must have the same filter shape and parameter units in
 * the live Tone graph and the FFmpeg composition graph. A similarly named
 * Tone/FFmpeg effect is not enough: algorithmically different implementations
 * (for example Tone PitchShift and FFmpeg rubberband) do not belong here.
 *
 * Keep the FFmpeg side in web-server/src/clip_editor.rs in sync. Its tests
 * assert the concrete filter parameters documented below.
 */
export const PARITY_APPROVED_EQ = {
	bass: {
		frequencyHz: 250,
		toneType: "lowshelf",
		ffmpegFilter: "bass",
		ffmpegWidthType: "slope",
		width: 1,
	},
	mid: {
		frequencyHz: 1_000,
		toneType: "peaking",
		ffmpegFilter: "equalizer",
		ffmpegWidthType: "q",
		width: 1,
	},
	treble: {
		frequencyHz: 3_000,
		toneType: "highshelf",
		ffmpegFilter: "treble",
		ffmpegWidthType: "slope",
		width: 1,
	},
} as const;

export type ParityApprovedEqId = keyof typeof PARITY_APPROVED_EQ;
