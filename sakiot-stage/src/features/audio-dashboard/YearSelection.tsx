import { FolderOpen as FolderOpenIcon } from "lucide-react";
import React from "react";
import { useLocation, useParams } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { useAppSelector } from "../../app/hooks";
import {
	Button,
	Drawer,
	Tab,
	TabList,
	TabPanel,
	Tabs,
	useMediaQuery,
} from "../../shared/ui";
import { ViewAsRoleBanner } from "../members/ViewAsRoleBanner";
import { AudioInterface } from "./AudioInterface";
import { LogicalSessionPlayer } from "./LogicalSessionPlayer";
import RecordingTree from "./TreeView";

export function YearSelection() {
	const params = useParams();
	const location = useLocation();
	const isDesktop = useMediaQuery("(min-width: 900px)");

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

	const tree = <RecordingTree onRecordingSelect={() => setTreeOpen(false)} />;

	return (
		<div className="flex flex-col w-full min-[900px]:h-full overflow-hidden">
			{!isDesktop && (
				<div className="p-2 shrink-0">
					<Button
						className="w-full"
						variant="outline"
						onPress={() => setTreeOpen(true)}
					>
						<FolderOpenIcon />
						Browse files
					</Button>
					<Drawer
						isOpen={treeOpen}
						side={"left"}
						onOpenChange={(isOpen) => {
							if (!isOpen) setTreeOpen(false);
						}}
					>
						<div className="w-70 p-2">{tree}</div>
					</Drawer>
				</div>
			)}
			<div className="p-2 min-[900px]:p-4 pb-0 shrink-0">
				<ViewAsRoleBanner guildId={params.guild_id ?? ""} />
			</div>
			<div className="flex flex-col min-[900px]:flex-row w-full min-w-0 flex-1 min-h-0 min-[900px]:h-full">
				{isDesktop && (
					<div
						className={
							"[flex:0_0_20%] min-w-55 max-w-80 h-full overflow-y-auto overflow-x-hidden [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
						}
					>
						{tree}
					</div>
				)}

				<div
					className={
						"flex-1 min-w-0 w-full min-[900px]:w-auto px-2 min-[900px]:px-4 pb-8 min-[900px]:h-full min-[900px]:overflow-y-auto overflow-x-hidden [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
					}
				>
					{params.session_id ? (
						<LogicalSessionPlayer
							key={params.session_id}
							sessionId={params.session_id}
						/>
					) : params.year ? (
						<Tabs
							className="mb-2 min-h-9"
							selectedKey={activeTab}
							onSelectionChange={(value) => {
								if (value === "normal" || value === "silence") setTab(value);
							}}
						>
							<TabList aria-label="View">
								<Tab className="min-h-9" id={"normal"}>
									Normal
								</Tab>
								{hasSilence && (
									<Tab className="min-h-9" id={"silence"}>
										Silence-free
									</Tab>
								)}
							</TabList>

							<TabPanel
								id="normal"
								shouldForceMount
								className="data-[inert]:hidden"
							>
								<AudioInterface
									key={`${location.pathname}-nosilence`}
									isClip={false}
									userGuilds={userGuilds}
									isSilence={false}
								/>
							</TabPanel>
							{hasSilence && (
								<TabPanel
									id="silence"
									shouldForceMount
									className="data-[inert]:hidden"
								>
									<AudioInterface
										key={`${location.pathname}-silence`}
										isClip={false}
										userGuilds={userGuilds}
										isSilence={true}
									/>
								</TabPanel>
							)}
						</Tabs>
					) : null}
				</div>
			</div>
		</div>
	);
}
