import type { components } from "../api/openapi";
import { API_ROUTES } from "../api/routes";
import { apiSlice } from "../app/apiSlice";
import { BASE_API_URL, refreshSession } from "../app/authedFetch";
import type { AppDispatch, RootState } from "../store";
import {
	type ApplyContext,
	applyChanged,
	applyPresence,
	reconcileGuild,
} from "./applyEvents";
import { type RealtimeStatus, setRealtimeStatus } from "./status";

type ServerMessage = components["schemas"]["ServerMessage"];
type ClientMessage = components["schemas"]["ClientMessage"];

export const PROTOCOL_VERSION = 1;
/** The server sends a heartbeat every 20 s; three missed means the link is dead. */
const STALE_AFTER_MS = 60_000;
const STALE_CHECK_MS = 5_000;
/** Renew the access token this long before it expires. */
const RENEW_BEFORE_EXPIRY_MS = 30_000;
const RENEW_RETRY_MS = 10_000;
/** Consecutive failed connections before polling takes over. */
const FALLBACK_AFTER_FAILURES = 3;
const CLOSE_TOKEN_EXPIRED = 4001;
const CLOSE_UNSUPPORTED_VERSION = 4400;
const CLOSE_SERVICE_RESTART = 1012;

export interface Scope {
	guildId: string;
	asRole?: string;
}

/** `ws(s)://<api host>/api/realtime`, from the configured API URL. */
export function realtimeUrl(
	base: string = BASE_API_URL,
	pageOrigin: string = window.location.origin,
): string {
	const url = new URL(API_ROUTES.realtime, new URL(base, pageOrigin).origin);
	url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
	return url.toString();
}

/** Exponential backoff with jitter: about 1 s, 2 s, 4 s … capped at 60 s. */
export function reconnectDelay(failures: number, random: () => number): number {
	const ceiling = Math.min(60_000, 1_000 * 2 ** Math.max(0, failures - 1));
	return ceiling / 2 + (random() * ceiling) / 2;
}

/** After a server restart (1012): spread reconnects over a few seconds. */
export function restartDelay(random: () => number): number {
	return 500 + random() * 2_500;
}

function sameScope(a: Scope | null, b: Scope | null): boolean {
	return a?.guildId === b?.guildId && a?.asRole === b?.asRole;
}

interface Connection {
	socket: WebSocket;
	/** Local time the server's `ready` arrived; 0 before. */
	readyAt: number;
	/** Local time the access token expires. */
	expiresAt: number;
	lastMessageAt: number;
	subscribed: boolean;
}

/**
 * Owns this tab's realtime socket: connection, scope, token renewal,
 * reconnects, and turning server messages into RTK Query updates. Status is
 * published through `status.ts` so pages poll only while realtime is not live.
 */
export class RealtimeCoordinator {
	private primary: Connection | null = null;
	private replacement: Connection | null = null;
	private scope: Scope | null = null;
	private generation = 0;
	private failures = 0;
	private running = false;
	private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
	private renewTimer: ReturnType<typeof setTimeout> | undefined;
	private staleTimer: ReturnType<typeof setInterval> | undefined;
	private readonly ctx: ApplyContext;

	constructor(
		dispatch: AppDispatch,
		getState: () => RootState,
		private readonly random: () => number = Math.random,
	) {
		this.ctx = { dispatch, getState, generation: () => this.generation };
	}

	start(): void {
		if (this.running) return;
		this.running = true;
		this.failures = 0;
		this.connect(false);
		this.staleTimer = setInterval(() => this.checkStale(), STALE_CHECK_MS);
		document.addEventListener("visibilitychange", this.onVisibilityChange);
	}

	/** Logout or account change: close everything. */
	stop(): void {
		this.running = false;
		this.generation += 1;
		clearTimeout(this.reconnectTimer);
		clearTimeout(this.renewTimer);
		clearInterval(this.staleTimer);
		document.removeEventListener("visibilitychange", this.onVisibilityChange);
		this.retire(this.primary);
		this.retire(this.replacement);
		this.primary = null;
		this.replacement = null;
		setRealtimeStatus("off");
	}

	/** The guild (and role preview) the current page shows, or none. */
	setScope(scope: Scope | null): void {
		if (sameScope(scope, this.scope)) return;
		this.scope = scope;
		this.generation += 1;
		const primary = this.primary;
		if (!primary || primary.readyAt === 0) return;
		primary.subscribed = false;
		if (scope) {
			this.status("connecting");
			this.subscribe(primary);
		} else {
			this.status("live");
		}
	}

	private status(next: RealtimeStatus): void {
		if (this.running) setRealtimeStatus(next);
	}

	private connect(asReplacement: boolean): void {
		if (!this.running) return;
		let socket: WebSocket;
		try {
			socket = new WebSocket(realtimeUrl());
		} catch {
			this.connectionFailed();
			return;
		}
		const connection: Connection = {
			socket,
			readyAt: 0,
			expiresAt: 0,
			lastMessageAt: Date.now(),
			subscribed: false,
		};
		if (asReplacement) {
			this.replacement = connection;
		} else {
			this.primary = connection;
			if (this.failures < FALLBACK_AFTER_FAILURES) this.status("connecting");
		}
		socket.onmessage = (event) => this.onMessage(connection, event);
		socket.onclose = (event) => this.onClose(connection, event.code);
	}

	private retire(connection: Connection | null): void {
		if (!connection) return;
		connection.socket.onmessage = null;
		connection.socket.onclose = null;
		connection.socket.close(1000);
	}

	private send(connection: Connection, message: ClientMessage): void {
		if (connection.socket.readyState === WebSocket.OPEN) {
			connection.socket.send(JSON.stringify(message));
		}
	}

	private subscribe(connection: Connection): void {
		const scope = this.scope;
		if (!scope) return;
		this.send(connection, {
			type: "set_scope",
			v: PROTOCOL_VERSION,
			guild_id: scope.guildId,
			...(scope.asRole ? { as_role: scope.asRole } : {}),
			// Who is in voice arrives as changes to apply, not as a signal for
			// every tab to refetch the whole list.
			presence_updates: true,
		});
	}

	private onMessage(connection: Connection, event: MessageEvent): void {
		connection.lastMessageAt = Date.now();
		let message: ServerMessage;
		try {
			message = JSON.parse(String(event.data)) as ServerMessage;
		} catch {
			return;
		}
		if (message.v !== PROTOCOL_VERSION) {
			this.unsupported();
			return;
		}
		switch (message.type) {
			case "ready": {
				const now = Date.now();
				connection.readyAt = now;
				connection.expiresAt =
					now + (message.token_expires_at - message.server_time);
				this.failures = 0;
				if (connection === this.primary) this.scheduleRenewal(connection);
				if (this.scope) {
					this.subscribe(connection);
				} else if (connection === this.replacement) {
					this.promote(connection);
				} else if (connection === this.primary) {
					this.status("live");
				}
				return;
			}
			case "subscribed": {
				const subscribedScope: Scope = {
					guildId: message.guild_id,
					...(message.as_role ? { asRole: message.as_role } : {}),
				};
				if (!sameScope(subscribedScope, this.scope)) return;
				connection.subscribed = true;
				if (connection === this.replacement) this.promote(connection);
				if (connection === this.primary) {
					this.status("live");
					// Anything that changed before this subscription took
					// effect, including across a reconnect, is refetched.
					reconcileGuild(this.ctx, message.guild_id);
				}
				return;
			}
			case "changed":
				if (message.guild_id === this.scope?.guildId) {
					applyChanged(this.ctx, message);
				}
				return;
			case "presence":
				if (this.scope && message.guild_id === this.scope.guildId) {
					applyPresence(this.ctx, message, this.scope.asRole);
				}
				return;
			case "access_changed":
				if (message.guild_id === this.scope?.guildId) {
					reconcileGuild(this.ctx, message.guild_id);
					// The guild list and admin flags may have changed too.
					this.ctx.dispatch(apiSlice.util.invalidateTags(["Auth"]));
				}
				return;
			case "resync_required":
				if (this.scope) reconcileGuild(this.ctx, this.scope.guildId);
				return;
			case "heartbeat":
				return;
		}
	}

	/** The renewed socket is subscribed: retire the old one. */
	private promote(connection: Connection): void {
		const previous = this.primary;
		this.primary = connection;
		this.replacement = null;
		this.retire(previous);
		this.scheduleRenewal(connection);
		if (!this.scope) this.status("live");
	}

	private onClose(connection: Connection, code: number): void {
		if (connection === this.replacement) {
			// The renewed socket failed; the current one serves until expiry.
			this.replacement = null;
			if (code === CLOSE_UNSUPPORTED_VERSION) this.unsupported();
			return;
		}
		if (connection !== this.primary) return;
		this.primary = null;
		clearTimeout(this.renewTimer);
		if (!this.running) return;
		if (code === CLOSE_UNSUPPORTED_VERSION) {
			this.unsupported();
		} else if (code === CLOSE_TOKEN_EXPIRED) {
			void this.refreshThenConnect();
		} else if (code === CLOSE_SERVICE_RESTART) {
			this.status("connecting");
			this.reconnectTimer = setTimeout(
				() => this.connect(false),
				restartDelay(this.random),
			);
		} else {
			this.connectionFailed();
		}
	}

	/**
	 * The socket failed or dropped. Never refreshes the token for this: a
	 * proxy that cannot upgrade, or a rejected origin, fails every time while
	 * HTTP works, and must not turn into a refresh loop.
	 */
	private connectionFailed(): void {
		this.failures += 1;
		if (this.failures >= FALLBACK_AFTER_FAILURES) this.status("fallback");
		clearTimeout(this.reconnectTimer);
		this.reconnectTimer = setTimeout(
			() => this.connect(false),
			reconnectDelay(this.failures, this.random),
		);
	}

	private scheduleRenewal(connection: Connection): void {
		clearTimeout(this.renewTimer);
		const delay = Math.max(
			0,
			connection.expiresAt - RENEW_BEFORE_EXPIRY_MS - Date.now(),
		);
		this.renewTimer = setTimeout(() => void this.renew(connection), delay);
	}

	/** Refresh before expiry, then open a replacement socket. */
	private async renew(connection: Connection): Promise<void> {
		if (!this.running || connection !== this.primary) return;
		const generation = this.generation;
		// Another tab may already have refreshed the shared cookies since this
		// socket's token was issued.
		const outcome = await refreshSession(connection.readyAt);
		if (generation !== this.generation || connection !== this.primary) return;
		if (outcome === "refreshed") {
			this.connect(true);
		} else if (outcome === "session-ended") {
			this.sessionEnded();
		} else if (Date.now() < connection.expiresAt) {
			this.renewTimer = setTimeout(
				() => void this.renew(connection),
				RENEW_RETRY_MS,
			);
		}
	}

	/** The server closed with 4001: the token expired before renewal. */
	private async refreshThenConnect(): Promise<void> {
		const generation = this.generation;
		const outcome = await refreshSession(Date.now() - RENEW_BEFORE_EXPIRY_MS);
		if (generation !== this.generation || !this.running) return;
		if (outcome === "refreshed") {
			this.connect(false);
		} else if (outcome === "session-ended") {
			this.sessionEnded();
		} else {
			this.failures += 1;
			if (this.failures >= FALLBACK_AFTER_FAILURES) this.status("fallback");
			this.reconnectTimer = setTimeout(
				() => void this.refreshThenConnect(),
				reconnectDelay(this.failures, this.random),
			);
		}
	}

	private sessionEnded(): void {
		this.stop();
		this.ctx.dispatch(apiSlice.util.invalidateTags(["Auth"]));
	}

	/** A newer server protocol: stop, keep drafts, prompt for a reload. */
	private unsupported(): void {
		this.stop();
		setRealtimeStatus("unsupported");
	}

	private checkStale(): void {
		const primary = this.primary;
		if (!primary) return;
		if (Date.now() - primary.lastMessageAt > STALE_AFTER_MS) {
			// Treated like any drop: reconnect with backoff.
			this.primary = null;
			this.retire(primary);
			this.connectionFailed();
		}
	}

	/** A suspended tab may hold a dead socket: check as soon as it shows. */
	private readonly onVisibilityChange = (): void => {
		if (document.visibilityState === "visible") this.checkStale();
	};
}
