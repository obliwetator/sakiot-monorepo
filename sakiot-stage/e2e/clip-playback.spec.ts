import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

test("Jam It reports a queued clip, then a cooldown with its delay", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of 429/);
	await mockClipEditorApi(page);
	const answers = [
		{ status: 200, body: { status: "queued" } },
		{
			status: 429,
			body: {
				code: 429,
				kind: "jam_cooldown",
				message: "You played a clip too recently. Try again in 7 seconds.",
				retry_after_seconds: 7,
			},
		},
	];
	const played: string[] = [];
	await page.route(
		`**/api/audio/clips/${GUILD_ID}/working-source/play`,
		async (route) => {
			if (route.request().method() === "OPTIONS") {
				await route.fulfill({ status: 204, headers: corsHeaders });
				return;
			}
			played.push(route.request().method());
			const answer = answers.shift();
			await route.fulfill({
				status: answer?.status ?? 500,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(answer?.body ?? {}),
			});
		},
	);

	await page.goto(`/dashboard/${GUILD_ID}/clips/working-source`);
	const jamIt = page.getByRole("button", { name: "Jam It" });
	const dialog = page.getByRole("dialog");

	await jamIt.click();
	await expect(dialog).toContainText("Clip queued");
	await dialog.getByRole("button", { name: "Close" }).click();
	await expect(dialog).toHaveCount(0);

	await jamIt.click();
	await expect(dialog).toContainText("On cooldown");
	await expect(dialog).toContainText("Try again in 7 seconds.");
	expect(played).toEqual(["POST", "POST"]);
});
