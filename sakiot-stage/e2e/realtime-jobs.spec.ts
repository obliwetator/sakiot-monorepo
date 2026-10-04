import type { WebSocketRoute } from "@playwright/test";
import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

const SOCKET_URL = "ws://127.0.0.1:4174/api/realtime";

test("a jobs event refreshes a building waveform without waiting for the poll", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	let requests = 0;
	let finished = false;
	await page.route(
		"**/api/audio/clips/waveform/guild-123/working-source*",
		async (route) => {
			requests += 1;
			await route.fulfill({
				status: finished ? 200 : 202,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(
					finished
						? { progress: 100 }
						: {
								id: "job-1",
								kind: "clip_waveform",
								status: "running",
								stage: "rendering",
								progress: 30,
							},
				),
			});
		},
	);
	// The server side: ready, then every scope is subscribed.
	let socket: WebSocketRoute | undefined;
	await page.routeWebSocket(SOCKET_URL, (ws) => {
		socket = ws;
		const now = Date.now();
		ws.send(
			JSON.stringify({
				type: "ready",
				v: 1,
				user_id: "current-user",
				server_time: now,
				token_expires_at: now + 15 * 60_000,
			}),
		);
		ws.onMessage((raw) => {
			const message = JSON.parse(String(raw)) as {
				type: string;
				guild_id?: string;
			};
			if (message.type === "set_scope") {
				ws.send(
					JSON.stringify({
						type: "subscribed",
						v: 1,
						guild_id: message.guild_id,
					}),
				);
			}
		});
	});

	await page.goto(`/dashboard/${GUILD_ID}/clips/working-source`);
	const building = page.getByText("Building clip waveform (30%)");
	await expect(building).toBeVisible();
	// A poll scheduled before realtime went live may still run once; after
	// that, the job is only checked every few seconds.
	await page.waitForTimeout(1_500);
	const settled = requests;
	await page.waitForTimeout(2_000);
	expect(requests).toBe(settled);

	// The job finishes: its event refetches the waveform at once.
	finished = true;
	socket?.send(
		JSON.stringify({
			type: "changed",
			v: 1,
			guild_id: GUILD_ID,
			resource: "jobs",
			ids: ["job-1"],
		}),
	);
	await expect.poll(() => requests, { timeout: 1_000 }).toBe(settled + 1);
	await expect(page.getByText(/Building clip waveform/)).toHaveCount(0);
});
