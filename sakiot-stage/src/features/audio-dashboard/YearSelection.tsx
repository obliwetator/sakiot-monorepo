import { FolderOpen as FolderOpenIcon } from "lucide-react";
import React from "react";
import { useLocation, useParams } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { useAppSelector } from "../../app/hooks";
import {
	Box,
	Button,
	Drawer,
	Tab,
	Tabs,
	useMediaQuery,
	useTheme,
} from "../../shared/ui";
import { ViewAsRoleBanner } from "../members/ViewAsRoleBanner";
import { AudioInterface } from "./AudioInterface";
import { LogicalSessionPlayer } from "./LogicalSessionPlayer";
import CustomizedTreeView from "./TreeView";

export function YearSelection() {
	const params = useParams();
	const location = useLocation();
	const theme = useTheme();
	const isDesktop = useMediaQuery(theme.breakpoints.up("md"));

	const hasSilence = useAppSelector((state) => state.hasSilence.value);
	const [tab, setTab] = React.useState<"normal" | "silence">("normal");
	// Silence tab only exists when a silence-free version is present; fall
	// back to normal whenever it isn't (e.g. navigating to another file).
	const activeTab = tab === "silence" && hasSilence ? "silence" : "normal";
	const [treeOpen, setTreeOpen] = React.useState(false);
	const { data: authData } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});
	const userGuilds = authData?.guilds || null;

	React.useEffect(() => {
		// A recording selection changes the route. Closing here also covers
		// selections initiated by keyboard or by a deep-link navigation.
		if (location.pathname) setTreeOpen(false);
	}, [location.pathname]);

	React.useEffect(() => {
		if (isDesktop) setTreeOpen(false);
	}, [isDesktop]);

	const tree = (
		<CustomizedTreeView onRecordingSelect={() => setTreeOpen(false)} />
	);

	return (
		<Box
			sx={{
				display: "flex",
				flexDirection: "column",
				width: "100%",
				height: { md: "100%" },
				overflow: "hidden",
			}}
		>
			{!isDesktop && (
				<Box sx={{ p: 1, flexShrink: 0 }}>
					<Button
						variant="outlined"
						fullWidth
						startIcon={<FolderOpenIcon />}
						onClick={() => setTreeOpen(true)}
					>
						Browse files
					</Button>
					<Drawer
						anchor="left"
						open={treeOpen}
						onClose={() => setTreeOpen(false)}
					>
						<Box sx={{ width: 280, p: 1 }}>{tree}</Box>
					</Drawer>
				</Box>
			)}
			<Box sx={{ p: { xs: 1, md: 2 }, pb: 0, flexShrink: 0 }}>
				<ViewAsRoleBanner guildId={params.guild_id ?? ""} />
			</Box>
			<Box
				sx={{
					display: "flex",
					flexDirection: { xs: "column", md: "row" },
					width: "100%",
					minWidth: 0,
					flex: 1,
					minHeight: 0,
					height: { md: "100%" },
				}}
			>
				{isDesktop && (
					<Box
						sx={{
							flex: "0 0 20%",
							minWidth: 220,
							maxWidth: 320,
							height: "100%",
							overflowY: "auto",
							overflowX: "hidden",
							// Hide scrollbar (Firefox / IE / WebKit)
							scrollbarWidth: "none",
							msOverflowStyle: "none",
							"&::-webkit-scrollbar": { display: "none" },
						}}
					>
						{tree}
					</Box>
				)}

				<Box
					sx={{
						flex: 1,
						minWidth: 0,
						width: { xs: "100%", md: "auto" },
						px: { xs: 1, md: 2 },
						pb: 4,
						height: { md: "100%" },
						overflowY: { md: "auto" },
						// Vertical scrolling must not implicitly turn this hidden-scrollbar
						// container into a horizontally pannable one.
						overflowX: "hidden",
						// Hide scrollbar (Firefox / IE / WebKit)
						scrollbarWidth: "none",
						msOverflowStyle: "none",
						"&::-webkit-scrollbar": { display: "none" },
					}}
				>
					{params.session_id ? (
						<LogicalSessionPlayer
							key={params.session_id}
							sessionId={params.session_id}
						/>
					) : params.year ? (
						<>
							<Tabs
								value={activeTab}
								onChange={(_e, v) => setTab(v)}
								sx={{ mb: 1, minHeight: 36 }}
							>
								<Tab label="Normal" value="normal" sx={{ minHeight: 36 }} />
								{hasSilence && (
									<Tab
										label="Silence-free"
										value="silence"
										sx={{ minHeight: 36 }}
									/>
								)}
							</Tabs>
							<Box sx={{ display: activeTab === "normal" ? "block" : "none" }}>
								<AudioInterface
									key={`${location.pathname}-nosilence`}
									isClip={false}
									userGuilds={userGuilds}
									isSilence={false}
								/>
							</Box>
							{hasSilence && (
								<Box
									sx={{ display: activeTab === "silence" ? "block" : "none" }}
								>
									<AudioInterface
										key={`${location.pathname}-silence`}
										isClip={false}
										userGuilds={userGuilds}
										isSilence={true}
									/>
								</Box>
							)}
						</>
					) : null}
				</Box>
			</Box>
		</Box>
	);
}
