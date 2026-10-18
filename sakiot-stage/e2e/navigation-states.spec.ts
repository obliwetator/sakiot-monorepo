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

test("a former member's server offers only their own recordings", async ({
	page,
}) => {
	await page.route("**/api/users/current/guilds", (route) =>
		route.fulfill({
			status: 200,
			headers: { ...corsHeaders, "Content-Type": "application/json" },
			body: JSON.stringify([
				{
					id: GUILD_ID,
					name: "Test Guild",
					icon: null,
					// Whatever it held as a member counts no more.
					capabilities: [],
					former: true,
				},
			]),
		}),
	);
	const presenceRequests: string[] = [];
	page.on("request", (request) => {
		if (request.url().includes("/voice-presence")) {
			presenceRequests.push(request.url());
		}
	});
	await page.goto("/dashboard");
	const server = page.getByRole("button", { name: /Test Guild/ });
	await expect(server).toContainText("Former member: your own recordings only");
	await server.click();
	await expect(page).toHaveURL(new RegExp(`/dashboard/${GUILD_ID}/audio$`));
	await expect(
		page.getByText("only recordings of your own voice are shown"),
	).toBeVisible();
	const navigation = page.getByRole("banner");
	for (const name of ["Audio", "Clips", "Stamps"]) {
		await expect(
			navigation.getByRole("button", { name, exact: true }),
		).toBeVisible();
	}
	await expect(
		navigation.getByRole("button", { name: "Clip Editor" }),
	).toHaveCount(0);
	// Who is in voice is a member's to see.
	await expect(
		page
			.getByRole("button", { name: "Browse files" })
			.or(page.getByRole("textbox", { name: "Search recordings" })),
	).toBeVisible();
	await page.waitForLoadState("networkidle");
	await expect(page.getByRole("heading", { name: "In voice" })).toHaveCount(0);
	expect(presenceRequests).toEqual([]);
});
