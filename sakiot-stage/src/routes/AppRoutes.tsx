import Box from "@mui/material/Box";
import React, { Suspense } from "react";
import { Route, Routes } from "react-router-dom";
import { LayoutsWithNavbar } from "../layouts/LayoutsWithNavbar";
import { ProtectedLayout } from "../layouts/ProtectedLayout";

const Clips = React.lazy(() => import("../features/clips"));
const YearSelection = React.lazy(() =>
	import("../features/audio-dashboard/YearSelection").then((m) => ({
		default: m.YearSelection,
	})),
);
const Stamps = React.lazy(() =>
	import("../features/stamps").then((m) => ({ default: m.Stamps })),
);
const GuildAdminCooldowns = React.lazy(() =>
	import("../features/admin-cooldowns").then((m) => ({
		default: m.GuildAdminCooldowns,
	})),
);
const GuildVoiceSettingsPage = React.lazy(() =>
	import("../features/admin-voice-settings").then((m) => ({
		default: m.GuildVoiceSettingsPage,
	})),
);

const lazyRoute = (node: React.ReactNode) => (
	<Suspense fallback={<Box p={2}>Loading Route</Box>}>{node}</Suspense>
);

export function AppRoutes() {
	return (
		<Routes>
			<Route path="/" element={<LayoutsWithNavbar />}>
				<Route path="/" element={<ProtectedLayout />} />
				<Route
					path=":guild_id"
					element={<Box p={2}>select from top navbar</Box>}
				/>

				<Route path="/stamps" element={lazyRoute(<Stamps />)} />
				<Route path="/stamps/:guild_id" element={lazyRoute(<Stamps />)} />

				<Route path="/dashboard" element={<ProtectedLayout />}>
					<Route path=":guild_id">
						<Route path="" element={<Box p={2}>select from top navbar</Box>} />
						<Route path="audio">
							<Route path="" element={lazyRoute(<YearSelection />)} />
							<Route
								path="session/:session_id"
								element={lazyRoute(<YearSelection />)}
							/>
							<Route
								path=":channel_id/:year/:month/:file_name"
								element={lazyRoute(<YearSelection />)}
							/>
						</Route>
						<Route path="clips">
							<Route path="" element={lazyRoute(<Clips />)} />
							<Route path=":file_name" element={lazyRoute(<Clips />)} />
						</Route>
						<Route path="admin">
							<Route
								path="cooldowns"
								element={lazyRoute(<GuildAdminCooldowns />)}
							/>
							<Route
								path="voice-settings"
								element={lazyRoute(<GuildVoiceSettingsPage />)}
							/>
						</Route>
					</Route>
				</Route>
			</Route>
		</Routes>
	);
}
