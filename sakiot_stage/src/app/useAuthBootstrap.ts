import { useEffect, useState } from "react";
import { useGetAuthDetailsQuery } from "./apiSlice";
import { BASE_API_URL, isLoggedIn as hasLoggedInCookie } from "./authedFetch";

export function useAuthBootstrap() {
	const [hasToken, setHasToken] = useState(hasLoggedInCookie());

	const {
		data: authData,
		isLoading,
		isError,
		refetch,
	} = useGetAuthDetailsQuery(undefined, {
		skip: !hasToken,
	});

	const isLoggedIn = !!authData?.user && !isError;

	useEffect(() => {
		const apiOrigin = new URL(BASE_API_URL, window.location.origin).origin;
		const handler = (e: MessageEvent) => {
			if (e.origin !== apiOrigin) return;
			if (e.data.success !== 1) {
				console.error("something failed when authenticating");
				return;
			}
			setHasToken(true);
			refetch();
			if (e.source && (e.source as Window).close) {
				setTimeout(() => (e.source as Window).close(), 200);
			}
		};
		window.addEventListener("message", handler);
		return () => window.removeEventListener("message", handler);
	}, [refetch]);

	return { authData, isLoading, isLoggedIn };
}
