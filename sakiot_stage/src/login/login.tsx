import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import type React from "react";
import { BASE_API_URL, useLogoutMutation } from "../app/apiSlice";
export default function Login(props: {
	isLoggedIn: boolean;
	setIsLoggedIn: React.Dispatch<React.SetStateAction<boolean>>;
}) {
	const [logout] = useLogoutMutation();

	const handleLogin = () => {
		const origin = encodeURIComponent(window.location.origin);
		window.open(
			`${BASE_API_URL}oauth/start?origin=${origin}`,
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
		props.setIsLoggedIn(false);
	};

	const isDevOrStaging =
		window.location.hostname === "localhost" ||
		window.location.hostname === "127.0.0.1" ||
		window.location.hostname.includes("staging") ||
		window.location.hostname.includes("dev");

	const handleDevLogin = async () => {
		const secret =
			(import.meta.env.VITE_DEV_LOGIN_SECRET as string | undefined) ||
			window.prompt("Dev login secret:") ||
			"";
		if (!secret) return;
		const res = await fetch(`${BASE_API_URL}dev_login?t=${Date.now()}`, {
			credentials: "include",
			headers: { "X-Dev-Login-Secret": secret },
		});
		if (!res.ok) {
			console.error("dev login failed", res.status);
			return;
		}
		window.location.reload();
	};

	return props.isLoggedIn ? (
		<Button
			onClick={() => {
				handleLogout();
			}}
			sx={{ my: 2, color: "white", display: "block" }}
		>
			Log out
		</Button>
	) : (
		<Box sx={{ display: "flex", gap: 2 }}>
			<Button
				onClick={() => {
					handleLogin();
				}}
				sx={{ my: 2, color: "white", display: "block" }}
			>
				Login
			</Button>
			{isDevOrStaging && (
				<Button
					onClick={() => {
						handleDevLogin();
					}}
					sx={{ my: 2, color: "white", display: "block" }}
				>
					Dev Login
				</Button>
			)}
		</Box>
	);
}
