import FolderOpenIcon from "@mui/icons-material/FolderOpen";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Drawer from "@mui/material/Drawer";
import { useTheme } from "@mui/material/styles";
import Tab from "@mui/material/Tab";
import Tabs from "@mui/material/Tabs";
import useMediaQuery from "@mui/material/useMediaQuery";
import React from "react";
import { useLocation, useParams } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { useAppSelector } from "../../app/hooks";
import { AudioInterface } from "./AudioInterface";
import CustomizedTreeView from "./TreeView";

export function YearSelection() {
	const params = useParams();
	const location = useLocation();

	const guildSelected = useAppSelector((state) => state.app.guildSelected);
	const hasSilence = useAppSelector((state) => state.hasSilence.value);
	const [tab, setTab] = React.useState<"normal" | "silence">("normal");
	// Silence tab only exists when a silence-free version is present; fall
	// back to normal whenever it isn't (e.g. navigating to another file).
	const activeTab = tab === "silence" && hasSilence ? "silence" : "normal";
	const { data: authData } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});
	const userGuilds = authData?.guilds || null;

	const theme = useTheme();
	const isDesktop = useMediaQuery(theme.breakpoints.up("md"));
	const [treeOpen, setTreeOpen] = React.useState(false);

	React.useEffect(() => {
		if (!isDesktop) setTreeOpen(false);
	}, [isDesktop]);

	const tree = <CustomizedTreeView guildSelected={guildSelected} />;

	return (
		<Box
			sx={{
				display: "flex",
				flexDirection: { xs: "column", md: "row" },
				width: "100%",
				height: { md: "100%" },
			}}
		>
			{isDesktop ? (
				<Box
					sx={{
						flex: "0 0 20%",
						minWidth: 220,
						maxWidth: 320,
						height: "100%",
						overflowY: "auto",
						// Hide scrollbar (Firefox / IE / WebKit)
						scrollbarWidth: "none",
						msOverflowStyle: "none",
						"&::-webkit-scrollbar": { display: "none" },
					}}
				>
					{tree}
				</Box>
			) : (
				<Box sx={{ p: 1 }}>
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
						<Box sx={{ width: 280 }}>{tree}</Box>
					</Drawer>
				</Box>
			)}

			<Box
				sx={{
					flex: 1,
					minWidth: 0,
					px: { xs: 1, md: 2 },
					height: { md: "100%" },
					overflowY: { md: "auto" },
					// Hide scrollbar (Firefox / IE / WebKit)
					scrollbarWidth: "none",
					msOverflowStyle: "none",
					"&::-webkit-scrollbar": { display: "none" },
				}}
			>
				{params.year && (
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
							<Box sx={{ display: activeTab === "silence" ? "block" : "none" }}>
								<AudioInterface
									key={`${location.pathname}-silence`}
									isClip={false}
									userGuilds={userGuilds}
									isSilence={true}
								/>
							</Box>
						)}
					</>
				)}
			</Box>
		</Box>
	);
}
