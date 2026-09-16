import { expect, test } from "@playwright/test";
import { corsHeaders, mockClipEditorApi } from "./clip-editor-fixture";

test("clip buffers survive effect replay and ignore a cleared source's pending decode", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	await page.route("**/api/audio/clips/guild-123/broken-source", (route) =>
		route.fulfill({ status: 200, headers: corsHeaders, body: "mock audio" }),
	);
	// Mount only the hook harness so unrelated app requests cannot update
	// another React root outside this test's act scope.
	await page.route("**/src/main.tsx", (route) =>
		route.fulfill({ contentType: "text/javascript", body: "" }),
	);
	await page.goto("/");
	const result = await page.evaluate(async () => {
		const reactUrl = "/node_modules/.vite/deps/react.js";
		const clientUrl = "/node_modules/.vite/deps/react-dom_client.js";
		const bufferUrl = "/src/features/clip-editor/useClipBuffer.ts";
		const { default: React } = await import(reactUrl);
		const { createRoot } = (await import(clientUrl)).default;
		const { useClipBuffer, loadClipBuffer } = await import(bufferUrl);
		const previousActEnvironment = Reflect.get(
			window,
			"IS_REACT_ACT_ENVIRONMENT",
		);
		Reflect.set(window, "IS_REACT_ACT_ENVIRONMENT", true);
		const host = document.createElement("div");
		document.body.append(host);
		const root = createRoot(host);
		const originalDecode = AudioContext.prototype.decodeAudioData;
		let setups = 0;
		function Harness({ clipId }: { clipId: string | null }) {
			const state = useClipBuffer("guild-123", clipId);
			React.useEffect(() => {
				setups += 1;
			}, []);
			return React.createElement("output", null, state.status);
		}
		const render = async (clipId: string | null) => {
			await React.act(async () => {
				root.render(
					React.createElement(
						React.StrictMode,
						null,
						React.createElement(Harness, { clipId }),
					),
				);
			});
			return host.textContent;
		};
		try {
			await render("working-source");
			let buffer!: AudioBuffer;
			await React.act(async () => {
				buffer = await loadClipBuffer("guild-123", "working-source");
			});
			const ready = host.textContent;
			let started!: () => void;
			const decoding = new Promise<void>((resolve) => {
				started = resolve;
			});
			let finishDecode!: (buffer: AudioBuffer) => void;
			AudioContext.prototype.decodeAudioData = (() =>
				new Promise<AudioBuffer>((resolve) => {
					finishDecode = resolve;
					started();
				})) as typeof originalDecode;
			await render("broken-source");
			await decoding;
			await render(null);
			await React.act(async () => {
				finishDecode(buffer);
				await loadClipBuffer("guild-123", "broken-source");
			});
			return { setups, ready, cleared: host.textContent };
		} finally {
			await React.act(async () => root.unmount());
			host.remove();
			AudioContext.prototype.decodeAudioData = originalDecode;
			Reflect.set(window, "IS_REACT_ACT_ENVIRONMENT", previousActEnvironment);
		}
	});
	expect(result).toEqual({ setups: 2, ready: "ready", cleared: "idle" });
});
