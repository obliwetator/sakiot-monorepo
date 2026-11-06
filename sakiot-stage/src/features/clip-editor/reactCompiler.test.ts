import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "@babel/core";
import reactCompiler, { type LoggerEvent } from "babel-plugin-react-compiler";

/**
 * The React Compiler skips a function it cannot compile without failing the
 * build, which silently drops its memoization. These files rely on being
 * compiled: they keep useCallback only for a function an effect depends on,
 * because Biome's dependency check cannot see the compiler.
 */
const COMPILED_HOOKS = [
	"ClipEditor.tsx",
	"SegmentWaveform.tsx",
	"Timeline.tsx",
	"useClipEditor.ts",
	"useCompositionExport.ts",
	"useDraftPersistence.ts",
	"useEditHistory.ts",
	"usePlaybackTransport.ts",
	"useProcessedSegmentWaveform.ts",
	"useSourceBuffers.ts",
	"useTimelineViewport.ts",
	"../../shared/MemberPicker.tsx",
	"../../shared/pointerDrag.ts",
	"../../shared/useDraftField.ts",
	"../audio-dashboard/AudioEventTimeline.tsx",
	"../audio-dashboard/ChannelMixPlayer.tsx",
	"../audio-dashboard/LogicalSessionPlayer.tsx",
	"../audio-dashboard/RangeSlider/useRangeSliderState.ts",
	"../audio-dashboard/TreeView/index.tsx",
	"../audio-dashboard/VoicePresencePanel.tsx",
	"../audio-dashboard/useChannelMixDraft.ts",
	"../audio-dashboard/useClipRangeViewport.ts",
	"../audio-dashboard/useSessionManifest.ts",
	"../audio-dashboard/useSessionSelectionController.ts",
];

describe("React Compiler coverage", () => {
	for (const file of COMPILED_HOOKS) {
		test(`${file} compiles`, () => {
			const path = fileURLToPath(new URL(file, import.meta.url));
			const events: LoggerEvent[] = [];
			transformSync(readFileSync(path, "utf8"), {
				filename: path,
				babelrc: false,
				configFile: false,
				parserOpts: { plugins: ["typescript", "jsx"] },
				plugins: [
					[
						reactCompiler,
						{
							panicThreshold: "none",
							logger: {
								logEvent: (_file: string | null, event: LoggerEvent) => {
									events.push(event);
								},
							},
						},
					],
				],
			});
			const bailouts = events.filter(
				(event) =>
					event.kind === "CompileError" ||
					event.kind === "CompileSkip" ||
					event.kind === "PipelineError",
			);
			expect(bailouts).toEqual([]);
			expect(events.some((event) => event.kind === "CompileSuccess")).toBe(
				true,
			);
		});
	}
});
