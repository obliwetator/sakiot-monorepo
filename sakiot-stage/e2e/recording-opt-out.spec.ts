import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

test("members can stop and resume being recorded in the selected server", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	// Registered last, so it takes precedence over the catch-all mock.
	let optedOut = false;
	const saved: unknown[] = [];
	await page.route(
		`**/api/users/current/guilds/${GUILD_ID}/recording-opt-out`,
		async (route) => {
			const request = route.request();
			const headers = {
				...corsHeaders,
				"Access-Control-Allow-Methods": "GET, PUT, OPTIONS",
			};
			if (request.method() === "OPTIONS") {
				await route.fulfill({ status: 204, headers });
				return;
			}
			if (request.method() === "PUT") {
				const body = request.postDataJSON() as { opted_out: boolean };
				saved.push(body);
				optedOut = body.opted_out;
			}
			await route.fulfill({
				status: 200,
				headers: { ...headers, "Content-Type": "application/json" },
				body: JSON.stringify({ opted_out: optedOut }),
			});
		},
	);

	await page.goto("/dashboard");
	await page.getByRole("button", { name: "Test Guild" }).click();
	await expect(page).toHaveURL(new RegExp(`/dashboard/${GUILD_ID}/audio$`));

	await page.getByRole("button", { name: "Open user menu" }).click();
	const recordMe = page.getByRole("menuitemcheckbox", {
		name: "Record my voice in Test Guild",
	});
	// Recording is on by default.
	await expect(recordMe).toBeChecked();

	await recordMe.click();
	await expect.poll(() => saved).toEqual([{ opted_out: true }]);
	await expect(recordMe).not.toBeChecked();

	await recordMe.click();
	await expect
		.poll(() => saved)
		.toEqual([{ opted_out: true }, { opted_out: false }]);
	await expect(recordMe).toBeChecked();
});
