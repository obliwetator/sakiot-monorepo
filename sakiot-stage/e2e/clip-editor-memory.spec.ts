import { expect, test } from "@playwright/test";
import { GUILD_ID, mockClipEditorApi } from "./clip-editor-fixture";

test("oversized browser preview is explained while export stays available", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	await page.addInitScript(
		({ guild }) => {
			localStorage.setItem(
				`sakiot:clip-editor:${guild}:draft`,
				JSON.stringify({
					master_volume_db: 0,
					muted_tracks: [],
					segments: [
						{
							source: "clip",
							source_id: "working-source",
							source_in: 0,
							source_out: 180,
							timeline_start: 0,
							track: 0,
							effects: {
								volume_db: 0,
								pitch_cents: 0,
								rate: 1,
								bass_db: 0,
								mid_db: 0,
								treble_db: 0,
								reverse: false,
							},
						},
					],
				}),
			);
		},
		{ guild: GUILD_ID },
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await expect(
		page.getByText("This edit is too large for browser preview.", {
			exact: false,
		}),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Play", exact: true }),
	).toBeDisabled();
	await page.keyboard.press("Space");
	await expect(
		page.getByRole("button", { name: "Pause", exact: true }),
	).toHaveCount(0);
	await expect(
		page.getByRole("button", { name: "Export", exact: true }),
	).toBeEnabled();
});
