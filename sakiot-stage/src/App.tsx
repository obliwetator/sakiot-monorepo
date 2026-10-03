import { type ReactNode, useEffect, useRef } from "react";
import {
	BrowserRouter,
	createBrowserRouter,
	createRoutesFromElements,
	RouterProvider,
} from "react-router-dom";
import { BundleUpdatePrompt } from "./app/BundleUpdatePrompt";
import { useAuthBootstrap } from "./app/useAuthBootstrap";
import { LayoutsWithNavbar } from "./layouts/LayoutsWithNavbar";
import {
	dropPreviousAccountData,
	useCrossTabLogin,
	useRealtime,
} from "./realtime/useRealtime";
import { appRoutesElement } from "./routes/AppRoutes";

// A data router so route-level hooks (useBlocker and friends) work; the route
// tree itself is the same declarative <Route> elements from AppRoutes.
const mainRouter = createBrowserRouter(
	createRoutesFromElements(appRoutesElement),
);
function AuthenticatedApp() {
	const { authData, isLoading, isLoggedIn } = useAuthBootstrap();
	const previousUser = useRef<string | null | undefined>(undefined);
	const userId = isLoggedIn ? authData?.user?.user_id : null;
	useEffect(() => {
		if (isLoading) return;
		const previous = previousUser.current;
		previousUser.current = userId;
		// An initial loader joins the auth probe itself. A later login/account
		// change must rerun loaders which previously had no authorized session,
		// and must not keep showing the previous account's data.
		if (previous !== undefined && userId && previous !== userId) {
			if (previous) dropPreviousAccountData();
			void mainRouter.revalidate();
		}
	}, [isLoading, userId]);
	useCrossTabLogin();
	// A missing `realtime_enabled` (an older server) means off.
	useRealtime(mainRouter, userId, authData?.user?.realtime_enabled === true);

	let content: ReactNode;
	if (isLoading || !isLoggedIn) {
		content = (
			<BrowserRouter>
				<LayoutsWithNavbar />
				<div className="p-4">
					{!isLoggedIn && !isLoading
						? "You are not logged in or you are not authorized to view this content"
						: "Loading Site"}
				</div>
				<BundleUpdatePrompt />
			</BrowserRouter>
		);
	} else {
		content = (
			<>
				<RouterProvider router={mainRouter} />
				<BundleUpdatePrompt />
				{authData?.user?.is_dev && (
					<div className="fixed bottom-4 right-4 bg-danger text-canvas px-2 py-1 rounded-[1px] font-bold z-9999 pointer-events-none">
						DEV ACCOUNT
					</div>
				)}
			</>
		);
	}

	return <>{content}</>;
}

export default AuthenticatedApp;
