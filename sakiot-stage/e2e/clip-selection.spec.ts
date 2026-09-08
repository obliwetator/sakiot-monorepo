import { expect, test } from "@playwright/test";
import { GUILD_ID, mockClipEditorApi } from "./clip-editor-fixture";

for (const interaction of ["title click", "keyboard"] as const) {
	test(`clip ${interaction} selects the clip and loads its player`, async ({
		page,
		isMobile,
	}) => {
		await mockClipEditorApi(page);
		await page.goto(`/dashboard/${GUILD_ID}/clips`);
		if (isMobile)
			await page
				.getByRole("button", { name: "Browse clips", exact: true })
				.click();
		const row = page.getByRole("button", { name: /Working source.*User/ });
		await expect(row).toBeVisible();
		if (interaction === "title click") {
			await row.getByText("Working source", { exact: true }).click();
		} else {
			await row.focus();
			await row.press("Enter");
		}
		await expect(page).toHaveURL(/\/clips\/working-source$/);
		await expect(
			page.getByRole("heading", { name: "Working source", exact: true }),
		).toBeVisible();
		await expect(
			page.getByRole("slider", { name: "Clip playback position" }),
		).toBeVisible();
		await expect(
			page.getByRole("button", { name: "Play", exact: true }),
		).toBeEnabled();
		if (isMobile) await expect(page.getByRole("dialog")).toHaveCount(0);
		else await expect(row).toHaveAttribute("aria-expanded", "true");
	});
}

test("clip search filters the list and restores it when cleared", async ({
	page,
	isMobile,
}) => {
	await mockClipEditorApi(page);
	await page.goto(`/dashboard/${GUILD_ID}/clips`);
	if (isMobile)
		await page
			.getByRole("button", { name: "Browse clips", exact: true })
			.click();

	const working = page.getByRole("button", { name: /Working source/ });
	const broken = page.getByRole("button", { name: /Broken source/ });
	await expect(working).toBeVisible();
	await expect(broken).toBeVisible();

	const search = page.getByRole("textbox", { name: "Search clips" });
	await search.fill("broken");
	await expect(broken).toBeVisible();
	await expect(working).toHaveCount(0);

	await search.fill("no-such-clip");
	await expect(page.getByText("No clips match this search.")).toBeVisible();
	await expect(working).toHaveCount(0);

	await search.fill("");
	await expect(working).toBeVisible();
	await expect(broken).toBeVisible();
});
