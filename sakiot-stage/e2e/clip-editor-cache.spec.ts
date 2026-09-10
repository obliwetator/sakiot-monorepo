import { expect, test } from "@playwright/test";
import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";

test("editor keeps timeline sources when the shared PCM cache evicts them", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	await page.route("**/api/audio/clips/guild-123/broken-source", (route) =>
		route.fulfill({ status: 200, headers: corsHeaders, body: "mock audio" }),
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips`);
	const result = await page.evaluate(async () => {
		// Exercise the real hook and shared cache. Large decoded buffers trigger
		// eviction without downloading large fixtures or depending on codecs.
		const reactUrl = "/node_modules/.vite/deps/react.js";
		const domUrl = "/node_modules/.vite/deps/react-dom_client.js";
		const hookUrl = "/src/features/clip-editor/useClipEditor.ts";
		const bufferUrl = "/src/features/clip-editor/useClipBuffer.ts";
		const { default: React } = await import(reactUrl);
		const { createRoot } = (await import(domUrl)).default;
		const { useClipEditor } = await import(hookUrl);
		const { loadClipBuffer } = await import(bufferUrl);
		const originalDecode = AudioContext.prototype.decodeAudioData;
		AudioContext.prototype.decodeAudioData = async function (
			this: AudioContext,
		) {
			return this.createBuffer(2, 5_000_000, 48_000);
		} as typeof originalDecode;
		const host = document.createElement("div");
		document.body.append(host);
		const root = createRoot(host);
		type Editor = {
			loadClip(
				guildId: string,
				clipId: string,
				length: number,
				track: number,
			): Promise<boolean>;
			sourceDuration(clipId: string): number | null;
		};
		let resolveReady!: (editor: Editor) => void;
		const ready = new Promise<Editor>((resolve) => {
			resolveReady = resolve;
		});
		function Harness() {
			const editor = useClipEditor();
			React.useEffect(() => {
				resolveReady(editor);
			}, []);
			return null;
		}
		try {
			root.render(React.createElement(Harness));
			const editor = await ready;
			await editor.loadClip("guild-123", "working-source", 1, 0);
			const before = editor.sourceDuration("working-source");
			// A second mocked source pushes retained
			// decoded PCM over 64 MiB, evicting the timeline's original source.
			await loadClipBuffer("guild-123", "broken-source");
			return { before, after: editor.sourceDuration("working-source") };
		} finally {
			root.unmount();
			host.remove();
			AudioContext.prototype.decodeAudioData = originalDecode;
		}
	});
	expect(result.before).toBeGreaterThan(0);
	expect(result.after).toBe(result.before);
});
