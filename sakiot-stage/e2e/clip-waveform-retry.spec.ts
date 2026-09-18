import { expect, test } from "@playwright/test";
import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";

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
}) => {
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
	await expect(retry).toBeEnabled({ timeout: 12000 });
	await retry.click();
	await expect
		.poll(() => requests, { timeout: 5000 })
		.toBeGreaterThanOrEqual(7);
});
