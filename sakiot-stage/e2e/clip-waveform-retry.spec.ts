import { expect, test } from "@playwright/test";
import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";

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
