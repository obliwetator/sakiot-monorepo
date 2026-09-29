import { Navigate, Route } from "react-router-dom";
import { GuildAccessBoundary } from "../layouts/GuildAccessBoundary";
import { LayoutsWithNavbar } from "../layouts/LayoutsWithNavbar";
import { ProtectedLayout } from "../layouts/ProtectedLayout";
import { audioRouteLoader } from "./audioRouteLoader";
import { shouldRevalidateGuildData } from "./guildRouteLoader";
import {
	clipsRouteLoader,
	cooldownsRouteLoader,
	membersRouteLoader,
	stampsRouteLoader,
	voiceSettingsRouteLoader,
} from "./pageRouteLoaders";
import { RouteLoadError } from "./RouteLoadError";
import { RouteState } from "./RouteState";

const loadClipsRoute = () =>
	import("../features/clips").then((m) => ({ Component: m.default }));
const loadClipEditorRoute = () =>
	import("../features/clip-editor").then((m) => ({ Component: m.default }));
const loadAudioRoute = () =>
	import("../features/audio-dashboard/YearSelection").then((m) => ({
		Component: m.YearSelection,
	}));
const loadStampsRoute = () =>
	import("../features/stamps").then((m) => ({ Component: m.Stamps }));
const loadCooldownsRoute = () =>
	import("../features/admin-cooldowns").then((m) => ({
		Component: m.GuildAdminCooldowns,
	}));
const loadVoiceSettingsRoute = () =>
	import("../features/admin-voice-settings").then((m) => ({
		Component: m.GuildVoiceSettingsPage,
	}));
const loadMembersRoute = () =>
	import("../features/members").then((m) => ({
		Component: m.GuildMembers,
	}));

// The route tree is a plain JSX element so the data router in App.tsx can
// consume it directly: createRoutesFromElements inspects Route/Fragment
// elements and never renders components.
// Keep error boundaries above lazy routes so failed imports can render recovery UI.
export const appRoutesElement = (
	<>
		<Route
			path="/"
			element={<LayoutsWithNavbar />}
			errorElement={<RouteLoadError />}
			hydrateFallbackElement={
				<p className="p-4" role="status">
					Loading Route
				</p>
			}
		>
			<Route index element={<ProtectedLayout />} />

			<Route path="/stamps" lazy={loadStampsRoute} />
			<Route
				path="/stamps/:guild_id"
				element={<GuildAccessBoundary />}
				loader={stampsRouteLoader}
				shouldRevalidate={shouldRevalidateGuildData}
				errorElement={<RouteLoadError page="stamps page" />}
			>
				<Route index lazy={loadStampsRoute} />
			</Route>

			<Route path="/dashboard">
				<Route index element={<ProtectedLayout />} />
				<Route path=":guild_id" element={<GuildAccessBoundary />}>
					<Route index element={<Navigate to="audio" replace />} />
					<Route
						path="audio"
						loader={audioRouteLoader}
						shouldRevalidate={shouldRevalidateGuildData}
						errorElement={<RouteLoadError page="audio page" />}
					>
						<Route index lazy={loadAudioRoute} />
						<Route path="session/:session_id" lazy={loadAudioRoute} />
						<Route
							path=":channel_id/:year/:month/:file_name"
							lazy={loadAudioRoute}
						/>
					</Route>
					<Route
						path="clips"
						loader={clipsRouteLoader}
						shouldRevalidate={shouldRevalidateGuildData}
						errorElement={<RouteLoadError page="clips page" />}
					>
						<Route index lazy={loadClipsRoute} />
						<Route path="editor" lazy={loadClipEditorRoute} />
						<Route path=":file_name" lazy={loadClipsRoute} />
					</Route>
					<Route
						path="admin"
						errorElement={<RouteLoadError page="settings page" />}
					>
						<Route
							path="cooldowns"
							lazy={loadCooldownsRoute}
							loader={cooldownsRouteLoader}
							shouldRevalidate={shouldRevalidateGuildData}
						/>
						<Route
							path="voice-settings"
							lazy={loadVoiceSettingsRoute}
							loader={voiceSettingsRouteLoader}
							shouldRevalidate={shouldRevalidateGuildData}
						/>
					</Route>
					<Route
						path="members"
						loader={membersRouteLoader}
						shouldRevalidate={shouldRevalidateGuildData}
						errorElement={<RouteLoadError page="members page" />}
					>
						<Route index lazy={loadMembersRoute} />
					</Route>
				</Route>
			</Route>
			<Route path="*" element={<RouteState kind="not-found" />} />
		</Route>
	</>
);
