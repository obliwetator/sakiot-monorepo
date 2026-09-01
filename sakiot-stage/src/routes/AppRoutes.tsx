import React, { Suspense } from "react";
import { Route } from "react-router-dom";
import { LayoutsWithNavbar } from "../layouts/LayoutsWithNavbar";
import { ProtectedLayout } from "../layouts/ProtectedLayout";
import { Box } from "../shared/ui";

const Clips = React.lazy(() => import("../features/clips"));
const ClipEditor = React.lazy(() => import("../features/clip-editor"));
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
const GuildMembers = React.lazy(() =>
	import("../features/members").then((m) => ({
		default: m.GuildMembers,
	})),
);

const lazyRoute = (node: React.ReactNode) => (
	<Suspense fallback={<Box p={2}>Loading Route</Box>}>{node}</Suspense>
);

// The route tree is a plain JSX element so the data router in App.tsx can
// consume it directly: createRoutesFromElements inspects Route/Fragment
// elements and never renders components.
export const appRoutesElement = (
	<>
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
						<Route path="editor" element={lazyRoute(<ClipEditor />)} />
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
					<Route path="members" element={lazyRoute(<GuildMembers />)} />
				</Route>
			</Route>
		</Route>
	</>
);
