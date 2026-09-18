import type React from "react";
import { useState } from "react";
import { API_ROUTES, apiAbsoluteUrl } from "../api/routes";
import { BASE_API_URL, useLogoutMutation } from "../app/apiSlice";
import { captureCsrfToken, setCsrfToken } from "../app/authedFetch";
import { Button } from "../shared/ui";
export default function Login(props: {
	isLoggedIn: boolean;
	setIsLoggedIn: React.Dispatch<React.SetStateAction<boolean>>;
}) {
	const [logout] = useLogoutMutation();
	const [devLoginPending, setDevLoginPending] = useState(false);
	const [devLoginError, setDevLoginError] = useState<string | null>(null);

	const handleLogin = () => {
		const origin = encodeURIComponent(window.location.origin);
		window.open(
			`${apiAbsoluteUrl(API_ROUTES.oauthStart)}?origin=${origin}`,
			"popup",
			"width=500,height=800",
		);
	};

	const handleLogout = async () => {
		try {
			await logout().unwrap();
		} catch (err) {
			console.error("logout request failed", err);
		}
		setCsrfToken(null);
		props.setIsLoggedIn(false);
	};

	const isDevOrStaging =
		window.location.hostname === "localhost" ||
		window.location.hostname === "127.0.0.1" ||
		window.location.hostname.includes("staging") ||
		window.location.hostname.includes("debug") ||
		window.location.hostname.includes("dev") ||
		window.location.hostname.includes("preview");

	const handleDevLogin = async () => {
		// Always prompt. A build-time VITE_DEV_LOGIN_SECRET would be compiled
		// into the public bundle, where anyone could read the secret that
		// mints dev sessions.
		const secret = window.prompt("Dev login secret:") ?? "";
		if (!secret) return;
		setDevLoginError(null);
		setDevLoginPending(true);
		// Not part of the documented API: the endpoint only exists in
		// dev-login builds, so it cannot come from API_ROUTES.
		try {
			const res = await fetch(
				`${BASE_API_URL}dev_login?t=${Date.now()}`, // raw-url-ok
				{
					credentials: "include",
					headers: { "X-Dev-Login-Secret": secret },
				},
			);
			if (!res.ok) {
				console.error("dev login failed", res.status);
				setDevLoginError(
					res.status === 403
						? "Invalid dev login secret."
						: res.status === 404
							? "Dev login is unavailable in this server build."
							: `Dev login failed (${res.status}).`,
				);
				return;
			}
			captureCsrfToken(res);
			window.location.reload();
		} catch (error) {
			console.error("dev login request failed", error);
			setDevLoginError(
				"Could not reach the local API. Start it with cargo dev up and try again.",
			);
		} finally {
			setDevLoginPending(false);
		}
	};

	return props.isLoggedIn ? (
		<Button
			className="my-2 rounded-sm border-0 px-2 text-sm font-medium uppercase tracking-normal text-white"
			variant="ghost"
			size="sm"
			onPress={() => {
				handleLogout();
			}}
		>
			Log out
		</Button>
	) : (
		<div>
			<div className="flex gap-4">
				<Button
					className="my-2 rounded-sm border-0 px-2 text-sm font-medium uppercase tracking-normal text-white"
					variant="ghost"
					size="sm"
					onPress={() => {
						handleLogin();
					}}
				>
					Login
				</Button>
				{isDevOrStaging && (
					<Button
						className="my-2 rounded-sm border-0 px-2 text-sm font-medium uppercase tracking-normal text-white"
						variant="ghost"
						size="sm"
						isDisabled={devLoginPending}
						onPress={() => {
							void handleDevLogin();
						}}
					>
						{devLoginPending ? "Signing in…" : "Dev Login"}
					</Button>
				)}
			</div>
			{devLoginError && (
				<p className="mb-2 max-w-72 text-sm text-danger" role="alert">
					{devLoginError}
				</p>
			)}
		</div>
	);
}
