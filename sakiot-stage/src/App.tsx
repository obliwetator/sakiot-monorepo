import type { ReactNode } from "react";
import {
	BrowserRouter,
	createBrowserRouter,
	createRoutesFromElements,
	RouterProvider,
} from "react-router-dom";
import { BundleUpdatePrompt } from "./app/BundleUpdatePrompt";
import { useAuthBootstrap } from "./app/useAuthBootstrap";
import { LayoutsWithNavbar } from "./layouts/LayoutsWithNavbar";
import { appRoutesElement } from "./routes/AppRoutes";

// A data router so route-level hooks (useBlocker and friends) work; the route
// tree itself is the same declarative <Route> elements from AppRoutes.
const mainRouter = createBrowserRouter(
	createRoutesFromElements(appRoutesElement),
);
function AuthenticatedApp() {
	const { authData, isLoading, isLoggedIn } = useAuthBootstrap();

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
					<div className="fixed bottom-4 right-4 bg-danger text-[#180b0b] px-2 py-1 rounded-[1px] font-bold z-9999 pointer-events-none">
						DEV ACCOUNT
					</div>
				)}
			</>
		);
	}

	return <>{content}</>;
}

export default AuthenticatedApp;
