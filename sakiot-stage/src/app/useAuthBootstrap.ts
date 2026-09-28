import { useEffect, useState } from "react";
import { apiSlice, useGetAuthDetailsQuery } from "./apiSlice";
import {
	BASE_API_URL,
	isLoggedIn as hasLoggedInCookie,
	setCsrfToken,
} from "./authedFetch";
import { useAppDispatch } from "./hooks";

export function useAuthBootstrap() {
	const [hasToken, setHasToken] = useState(hasLoggedInCookie());
	const dispatch = useAppDispatch();

	const {
		data: authData,
		isLoading,
		isError,
	} = useGetAuthDetailsQuery(undefined, {
		skip: !hasToken,
	});

	const isLoggedIn = !!authData?.user && !isError;

	useEffect(() => {
		const apiOrigin = new URL(BASE_API_URL, window.location.origin).origin;
		const handler = (e: MessageEvent) => {
			if (e.origin !== apiOrigin) return;
			if (e.data?.type !== "sakiot-auth" || e.data?.success !== 1) return;
			if (typeof e.data.csrf !== "string" || e.data.csrf.length < 16) return;
			setCsrfToken(e.data.csrf);
			setHasToken(true);
			// A same-origin logged-out page may have skipped the initial query;
			// initiate works in that case too, and the hook joins this request.
			void dispatch(
				apiSlice.endpoints.getAuthDetails.initiate(undefined, {
					subscribe: false,
					forceRefetch: true,
				}),
			);
			if (e.source && (e.source as Window).close) {
				setTimeout(() => (e.source as Window).close(), 200);
			}
		};
		window.addEventListener("message", handler);
		return () => window.removeEventListener("message", handler);
	}, [dispatch]);

	return { authData, isLoading, isLoggedIn };
}
