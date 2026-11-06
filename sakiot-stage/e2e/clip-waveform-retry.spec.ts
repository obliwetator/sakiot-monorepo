import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

test("a terminal waveform error stops polling until manual retry", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	let requests = 0;
	let retried = false;
	await page.route(
		"**/api/audio/clips/waveform/guild-123/working-source*",
		async (route) => {
			requests++;
			await route.fulfill({
				status: 200,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(
					retried
						? { progress: 100 }
						: { progress: 0, error: "Waveform generation failed" },
				),
			});
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/working-source`);
	const retry = page.getByRole("button", { name: "Retry waveform" });
	await expect(retry).toBeEnabled();
	await page.waitForTimeout(2_200);
	expect(requests).toBe(1);

	retried = true;
	await retry.click();
	await expect
		.poll(() => requests, { timeout: 5_000 })
		.toBeGreaterThanOrEqual(2);
	await expect(retry).toHaveCount(0);
});

test("manual retry resumes waveform polling after exhausted errors", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(
		/status of 500.*\/api\/audio\/clips\/waveform\/guild-123\/working-source/,
	);
	await mockClipEditorApi(page);
	let requests = 0;
	await page.route(
		"**/api/audio/clips/waveform/guild-123/working-source*",
		async (route) => {
			requests++;
			await route.fulfill({
				status: requests <= 5 ? 500 : 200,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(
					requests <= 5
						? { message: "temporary failure" }
						: { progress: requests === 6 ? 40 : 100 },
				),
			});
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/working-source`);
	const retry = page.getByRole("button", { name: "Retry waveform" });
	// Five failed polls a second apart, after the page loads.
	await expect(retry).toBeEnabled({ timeout: 20_000 });
	await retry.click();
	await expect
		.poll(() => requests, { timeout: 8_000 })
		.toBeGreaterThanOrEqual(7);
});

test("failed polls count even when each settles within a frame", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(
		/status of 500.*\/api\/audio\/clips\/waveform\/guild-123\/working-source/,
	);
	// RTK Query tells subscribers about a request on the next animation frame.
	// With slow frames a response always beats it, so a component never sees
	// the request in flight: what a network failing instantly looks like.
	await page.addInitScript(() => {
		const nextFrame = window.requestAnimationFrame.bind(window);
		window.requestAnimationFrame = (callback) =>
			window.setTimeout(() => nextFrame(callback), 200);
	});
	await mockClipEditorApi(page);
	await page.route(
		"**/api/audio/clips/waveform/guild-123/working-source*",
		async (route) => {
			await route.fulfill({
				status: 500,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify({ message: "temporary failure" }),
			});
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/working-source`);
	await expect(
		page.getByRole("button", { name: "Retry waveform" }),
	).toBeEnabled({ timeout: 20_000 });
});
