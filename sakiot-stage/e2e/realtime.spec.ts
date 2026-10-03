import type { Page, Route, WebSocketRoute } from "@playwright/test";
import { expect, fulfillRecordingOptOut, test } from "./fixtures";

/**
 * Realtime on: the mocked server reports `realtime_enabled`, and
 * `page.routeWebSocket` plays the `/api/realtime` side of the protocol.
 */

const API_ORIGIN = "http://127.0.0.1:4174";
const SOCKET_URL = "ws://127.0.0.1:4174/api/realtime";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

type Handler = (
	path: string,
	route: Route,
	json: (body: unknown, status?: number) => Promise<void>,
) => Promise<boolean>;

async function mockApi(page: Page, handler: Handler) {
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (await fulfillRecordingOptOut(route, corsHeaders)) return;
		const json = async (body: unknown, status = 200) => {
			await route.fulfill({
				status,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		};
		if (path === "/api/users/current") {
			await json({
				avatar: "",
				is_dev: false,
				realtime_enabled: true,
				user_id: "current-user",
				username: "Test Admin",
			});
			return;
		}
		if (path === "/api/users/current/guilds") {
			await json([
				{ id: GUILD_ID, name: "Test Guild", owner: true, permissions: "8" },
			]);
			return;
		}
		if (await handler(path, route, json)) return;
		await json(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});
}

const now = Date.now();

/** A server that says `ready`, then subscribes whatever scope is asked for. */
function serve(ws: WebSocketRoute, onScope?: (ws: WebSocketRoute) => void) {
	ws.send(
		JSON.stringify({
			type: "ready",
			v: 1,
			user_id: "current-user",
			server_time: now,
			token_expires_at: now + 15 * 60_000,
		}),
	);
	ws.onMessage((raw) => {
		const message = JSON.parse(String(raw)) as {
			type: string;
			guild_id?: string;
			as_role?: string;
		};
		if (message.type !== "set_scope") return;
		ws.send(
			JSON.stringify({
				type: "subscribed",
				v: 1,
				guild_id: message.guild_id,
				...(message.as_role ? { as_role: message.as_role } : {}),
			}),
		);
		onScope?.(ws);
	});
}

function changed(resource: string, ids?: string[]) {
	return JSON.stringify({
		type: "changed",
		v: 1,
		guild_id: GUILD_ID,
		resource,
		...(ids ? { ids } : {}),
	});
}

test("a settings refresh keeps the field being edited and updates the rest", async ({
	page,
}) => {
	let pendingCap = 21_600;
	let retention: number | null = null;
	await mockApi(page, async (path, route, json) => {
		const method = route.request().method();
		if (path === `/api/admin/guilds/${GUILD_ID}/voice-settings`) {
			await json({ pending_cap_seconds: pendingCap, is_default: false });
			return method === "GET";
		}
		if (path === `/api/admin/guilds/${GUILD_ID}/recording-policy`) {
			await json({
				retention_days: retention,
				excluded_channel_ids: [],
				channels: [],
				is_default: retention === null,
			});
			return method === "GET";
		}
		return false;
	});
	let socket: WebSocketRoute | null = null;
	await page.routeWebSocket(SOCKET_URL, (ws) => {
		socket = ws;
		serve(ws);
	});

	await page.goto(`/dashboard/${GUILD_ID}/admin/voice-settings`);
	const cap = page.getByRole("spinbutton", { name: "Pending cap (seconds)" });
	await expect(cap).toHaveValue("21600");
	await expect.poll(() => socket !== null).toBe(true);
	await cap.fill("3600");

	// Another admin saves both settings; the server pushes the change.
	pendingCap = 7_200;
	retention = 45;
	const ws = socket as unknown as WebSocketRoute;
	ws.send(changed("voice_settings"));
	ws.send(changed("recording_policy"));

	await expect(
		page.getByRole("spinbutton", { name: "Hide recordings after (days)" }),
	).toHaveValue("45");
	await expect(cap).toHaveValue("3600");
	await expect(
		page.getByText("Someone else saved this while you were editing."),
	).toBeVisible();
	await page.getByRole("button", { name: "Use saved value" }).click();
	await expect(cap).toHaveValue("7200");
	await expect(page.getByText(/Someone else saved/)).toHaveCount(0);
});

test("a recordings event patches one session into the tree", async ({
	page,
}) => {
	const fileEntry = (session: string, name: string) => ({
		file: `${session}.ogg`,
		display_name: name,
		recording_session_id: session,
		state: "finalized",
		user_id: "user-123",
	});
	const requests: string[] = [];
	page.on("request", (request) => {
		const url = new URL(request.url());
		if (url.pathname.startsWith(`/api/current/${GUILD_ID}`))
			requests.push(url.pathname);
	});
	await mockApi(page, async (path, _route, json) => {
		if (path === `/api/current/${GUILD_ID}`) {
			await json([
				{
					channel_id: "100",
					dirs: [{ year: 2026, months: { 8: [fileEntry("1", "Alice")] } }],
				},
			]);
			return true;
		}
		if (path === `/api/current/${GUILD_ID}/live-stems`) {
			await json([]);
			return true;
		}
		if (path === `/api/current/${GUILD_ID}/sessions/2`) {
			await json({
				channel_id: "100",
				dirs: [{ year: 2026, months: { 8: [fileEntry("2", "Bob")] } }],
			});
			return true;
		}
		return false;
	});
	let subscribed = false;
	let socket: WebSocketRoute | null = null;
	await page.routeWebSocket(SOCKET_URL, (ws) => {
		socket = ws;
		serve(ws, () => {
			subscribed = true;
		});
	});

	await page.goto(`/dashboard/${GUILD_ID}/audio`);
	// Narrow layouts hide the tree behind "Browse files".
	const browse = page.getByRole("button", { name: "Browse files" });
	const tree = page.getByRole("treegrid", { name: "Recordings" });
	await expect(browse.or(tree).first()).toBeVisible();
	if (await browse.isVisible()) await browse.click();
	await expect(tree).toBeVisible();
	await expect.poll(() => subscribed).toBe(true);
	// `subscribed` reconciles once; wait for that refetch to settle.
	await expect
		.poll(() => requests.filter((p) => p === `/api/current/${GUILD_ID}`).length)
		.toBeGreaterThanOrEqual(2);
	const treeLoads = requests.filter(
		(p) => p === `/api/current/${GUILD_ID}`,
	).length;

	(socket as unknown as WebSocketRoute).send(changed("recordings", ["2"]));

	await expect
		.poll(() => requests.includes(`/api/current/${GUILD_ID}/sessions/2`))
		.toBe(true);
	for (const row of ["2026", "August"]) {
		const header = tree.getByRole("row", { name: row, exact: true });
		if ((await header.getAttribute("aria-expanded")) === "false") {
			await header.click();
		}
	}
	await expect(tree.getByText("Bob")).toBeVisible();
	await expect(tree.getByText("Alice")).toBeVisible();
	// One session was fetched; the tree itself was not reloaded.
	expect(requests.filter((p) => p === `/api/current/${GUILD_ID}`).length).toBe(
		treeLoads,
	);
});

test("an unsupported protocol version asks for a reload", async ({ page }) => {
	await mockApi(page, async (path, _route, json) => {
		if (path === `/api/stamps/${GUILD_ID}`) {
			await json([]);
			return true;
		}
		return false;
	});
	await page.routeWebSocket(SOCKET_URL, (ws) => {
		ws.close({ code: 4400, reason: "unsupported protocol version" });
	});
	await page.goto(`/stamps/${GUILD_ID}`);
	await expect(page.getByText("New version available")).toBeVisible();
});

test("a socket that keeps failing falls back to polling", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/WebSocket/);
	await page.clock.install();
	let stampLoads = 0;
	let refreshes = 0;
	await mockApi(page, async (path, _route, json) => {
		if (path === `/api/stamps/${GUILD_ID}`) {
			stampLoads += 1;
			await json([]);
			return true;
		}
		if (path === "/api/refresh") {
			refreshes += 1;
			await json({ status: "ok" });
			return true;
		}
		return false;
	});
	let attempts = 0;
	await page.routeWebSocket(SOCKET_URL, (ws) => {
		attempts += 1;
		ws.close({ code: 1011, reason: "proxy refused" });
	});

	await page.goto(`/stamps/${GUILD_ID}`);
	// The page (and its polling query) must be mounted before time advances.
	await expect(page.getByText("No stamps yet.")).toBeVisible();
	expect(stampLoads).toBe(1);
	// Backoff retries (about 1 s, 2 s, 4 s …) never touch the token, and the
	// page keeps polling every 30 s meanwhile.
	// Advance in steps: the 30 s poll is scheduled only once the first
	// response has been handled.
	for (let step = 0; step < 20 && stampLoads < 2; step += 1) {
		await page.clock.runFor(5_000);
	}
	expect(stampLoads).toBeGreaterThanOrEqual(2);
	expect(attempts).toBeGreaterThanOrEqual(3);
	// A socket that fails while HTTP works never turns into token refreshes.
	expect(refreshes).toBe(0);
});
