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
import { Box } from "./shared/ui";

// A data router so route-level hooks (useBlocker and friends) work; the route
// tree itself is the same declarative <Route> elements from AppRoutes.
const mainRouter = createBrowserRouter(
	createRoutesFromElements(appRoutesElement),
);

function App() {
	const { authData, isLoading, isLoggedIn } = useAuthBootstrap();

	let content: ReactNode;
	if (isLoading || !isLoggedIn) {
		content = (
			<BrowserRouter>
				<LayoutsWithNavbar />
				<Box p={2}>
					{!isLoggedIn && !isLoading
						? "You are not logged in or you are not authorized to view this content"
						: "Loading Site"}
				</Box>
				<BundleUpdatePrompt />
			</BrowserRouter>
		);
	} else {
		content = (
			<>
				<RouterProvider router={mainRouter} />
				<BundleUpdatePrompt />
				{authData?.user?.is_dev && (
					<Box
						sx={{
							position: "fixed",
							bottom: 16,
							right: 16,
							backgroundColor: "error.main",
							color: "error.contrastText",
							padding: "4px 8px",
							borderRadius: 1,
							fontWeight: "bold",
							zIndex: 9999,
							pointerEvents: "none",
						}}
					>
						DEV ACCOUNT
					</Box>
				)}
			</>
		);
	}

	return <>{content}</>;
}

export default App;
