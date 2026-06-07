import Alert from "@mui/material/Alert";
import Button from "@mui/material/Button";
import Snackbar from "@mui/material/Snackbar";
import { useCallback, useEffect, useRef, useState } from "react";

const CURRENT_BUNDLE_VERSION = __BUNDLE_VERSION__;
const VERSION_URL = "/version.json";
const VERSION_CHECK_INTERVAL_MS = 5 * 60 * 1000;

async function fetchBundleVersion(): Promise<string | null> {
	const response = await fetch(`${VERSION_URL}?t=${Date.now()}`, {
		cache: "no-store",
		headers: { Accept: "application/json" },
	});

	if (!response.ok) return null;

	const data: unknown = await response.json();
	if (
		typeof data === "object" &&
		data !== null &&
		"version" in data &&
		typeof data.version === "string"
	) {
		return data.version;
	}

	return null;
}

function useBundleUpdateAvailable() {
	const currentBundleVersionRef = useRef(CURRENT_BUNDLE_VERSION);
	const [updateAvailable, setUpdateAvailable] = useState(false);

	const checkVersion = useCallback(async () => {
		try {
			const version = await fetchBundleVersion();
			if (version === null || updateAvailable) return;

			if (currentBundleVersionRef.current !== version) {
				setUpdateAvailable(true);
			}
		} catch {
			// Ignore transient network errors. The next poll will retry.
		}
	}, [updateAvailable]);

	useEffect(() => {
		void checkVersion();

		const intervalId = window.setInterval(() => {
			void checkVersion();
		}, VERSION_CHECK_INTERVAL_MS);

		const handleVisibilityChange = () => {
			if (document.visibilityState === "visible") {
				void checkVersion();
			}
		};

		window.addEventListener("focus", checkVersion);
		document.addEventListener("visibilitychange", handleVisibilityChange);

		return () => {
			window.clearInterval(intervalId);
			window.removeEventListener("focus", checkVersion);
			document.removeEventListener("visibilitychange", handleVisibilityChange);
		};
	}, [checkVersion]);

	return updateAvailable;
}

export function BundleUpdatePrompt() {
	const updateAvailable = useBundleUpdateAvailable();

	return (
		<Snackbar
			open={updateAvailable}
			anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
		>
			<Alert
				severity="info"
				variant="filled"
				action={
					<Button
						color="inherit"
						size="small"
						onClick={() => window.location.reload()}
					>
						Reload
					</Button>
				}
				sx={{ alignItems: "center" }}
			>
				New version available
			</Alert>
		</Snackbar>
	);
}
