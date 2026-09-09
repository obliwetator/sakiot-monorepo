import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";

/** The login buttons live in the nav drawer on the mobile viewport. */
async function openDevLogin(page: Page, projectName: string) {
	if (projectName === "mobile-chromium") {
		await page.getByRole("button", { name: "open navigation" }).click();
	}
	const devLogin = page.getByRole("button", { name: "Dev Login" });
	await expect(devLogin).toBeVisible();
	return devLogin;
}

test("prompts for the dev-login secret and sends it as X-Dev-Login-Secret", async ({
	page,
}, testInfo) => {
	// The secret must come from the prompt, never from a build-time
	// VITE_DEV_LOGIN_SECRET baked into the public bundle.
	const devLoginHeaders: Array<Record<string, string>> = [];
	const dialogs: string[] = [];
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		if (url.pathname === "/api/dev_login") {
			devLoginHeaders.push(request.headers());
			await route.fulfill({
				status: 204,
				headers: { "X-CSRF-Token": "e2e-csrf-token-1234567890" },
			});
			return;
		}
		// Every other call is unauthenticated, so the login shell stays up.
		await route.fulfill({ status: 401, body: "" });
	});
	page.on("dialog", async (dialog) => {
		dialogs.push(dialog.type());
		await dialog.accept("prompted-secret");
	});

	await page.goto("/");
	const devLogin = await openDevLogin(page, testInfo.project.name);
	await devLogin.click();

	await expect.poll(() => devLoginHeaders.length).toBeGreaterThan(0);
	expect(dialogs).toContain("prompt");
	expect(devLoginHeaders[0]["x-dev-login-secret"]).toBe("prompted-secret");
});

test("a dismissed dev-login prompt sends no request", async ({
	page,
}, testInfo) => {
	const devLoginRequests: string[] = [];
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		if (url.pathname === "/api/dev_login") {
			devLoginRequests.push(url.pathname);
			await route.fulfill({ status: 204 });
			return;
		}
		await route.fulfill({ status: 401, body: "" });
	});
	page.on("dialog", (dialog) => dialog.dismiss());

	await page.goto("/");
	const devLogin = await openDevLogin(page, testInfo.project.name);
	await devLogin.click();

	await expect(page.getByText("You are not logged in")).toBeVisible();
	expect(devLoginRequests).toEqual([]);
});
