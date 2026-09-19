import {
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

test.beforeEach(async ({ page }) => {
	await mockClipEditorApi(page);
});

test("server selection opens audio instead of a placeholder", async ({
	page,
}) => {
	await page.goto("/dashboard");
	await expect(
		page.getByRole("heading", { name: "Choose a server" }),
	).toBeVisible();
	await page.getByRole("button", { name: "Test Guild" }).click();
	await expect(page).toHaveURL(new RegExp(`/dashboard/${GUILD_ID}/audio$`));
	await expect(
		page
			.getByRole("button", { name: "Browse files" })
			.or(page.getByRole("textbox", { name: "Search recordings" })),
	).toBeVisible();
});

test("unknown server routes show a useful forbidden state", async ({
	page,
}) => {
	await page.goto("/dashboard/unknown-guild/audio");
	await expect(
		page.getByRole("heading", { name: "Server access unavailable" }),
	).toBeVisible();
	await page.getByRole("link", { name: "Choose a server" }).click();
	await expect(
		page.getByRole("heading", { name: "Choose a server" }),
	).toBeVisible();
});

test("unknown URLs show a not-found state", async ({ page }) => {
	await page.goto("/not-a-real-page");
	await expect(
		page.getByRole("heading", { name: "Page not found" }),
	).toBeVisible();
});

test("accounts with no servers get an empty state", async ({ page }) => {
	await page.route("**/api/users/current/guilds", (route) =>
		route.fulfill({
			status: 200,
			headers: { ...corsHeaders, "Content-Type": "application/json" },
			body: "[]",
		}),
	);
	await page.goto("/dashboard");
	await expect(
		page.getByRole("heading", { name: "No servers available" }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Refresh servers" }),
	).toBeVisible();
});
