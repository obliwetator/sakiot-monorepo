import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "@babel/core";
import reactCompiler, { type LoggerEvent } from "babel-plugin-react-compiler";

/**
 * The React Compiler skips a function it cannot compile without failing the
 * build, which silently drops its memoization. These hooks carry no manual
 * useCallback/useMemo: they rely on being compiled.
 */
const COMPILED_HOOKS = [
	"useClipEditor.ts",
	"useDraftPersistence.ts",
	"usePlaybackTransport.ts",
	"useSourceBuffers.ts",
	"useTimelineViewport.ts",
	"../../shared/useDraftField.ts",
	"../admin-cooldowns/MemberPicker.tsx",
	"../audio-dashboard/VoicePresencePanel.tsx",
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
