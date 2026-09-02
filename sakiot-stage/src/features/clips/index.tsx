import {
	Scissors as ContentCutIcon,
	ChevronDown as ExpandMoreIcon,
	Film as MovieIcon,
} from "lucide-react";
import type React from "react";
import { useEffect, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import {
	type ClipData,
	useDeleteClipMutation,
	useGetAuthDetailsQuery,
	useGetClipsQuery,
} from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { useAppSelector } from "../../app/hooks";
import { useAsRole } from "../../app/useAsRole";
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../../Constants";
import { canDeleteClip } from "../../shared/permissions";
import {
	Accordion,
	AccordionDetails,
	AccordionSummary,
	Box,
	Button,
	Dialog,
	DialogActions,
	DialogContent,
	DialogContentText,
	DialogTitle,
	Drawer,
	Page,
	Stack,
	Tab,
	Tabs,
	Text,
	Typography,
	useMediaQuery,
	useTheme,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { ViewAsRoleBanner } from "../members/ViewAsRoleBanner";
import { ClipPlayer } from "./ClipPlayer";
import { isComposedClip } from "./composedClip";

function SimpleAccordion(props: {
	data: ClipData[];
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const navigate = useNavigate();
	const location = useLocation();
	const [expanded, setExpanded] = useState<string | false>(false);

	const handleClickAccordion = (guild_id: string, clip_id: string) => {
		const clipPath = `${PATH_PREFIX_FOR_LOGGED_USERS}/${guild_id}/clips/${encodeURIComponent(clip_id)}`;
		if (location.pathname !== clipPath) {
			navigate(clipPath);
		}
	};

	const handleChange =
		(panel: string) => (_event: React.SyntheticEvent, isExpanded: boolean) => {
			setExpanded(isExpanded ? panel : false);
		};

	const elements = props.data.map((el, index) => {
		return (
			<Accordion
				key={el.clip_id}
				disableGutters
				sx={{
					mb: 1,
					border: "1px solid",
					borderColor: location.pathname.endsWith(
						encodeURIComponent(el.clip_id),
					)
						? "secondary.main"
						: "divider",
					borderRadius: "8px !important",
					boxShadow: "none",
					"&:before": { display: "none" },
				}}
				onClick={() => {
					handleClickAccordion(el.guild_id, el.clip_id);
				}}
				onChange={handleChange(`panel${index}`)}
				expanded={expanded === `panel${index}`}
			>
				<AccordionSummary
					expandIcon={<ExpandMoreIcon />}
					aria-controls="panel1a-content"
					id="panel1a-header"
				>
					<Box sx={{ minWidth: 0, flex: 1 }}>
						<Typography sx={{ overflowWrap: "anywhere" }}>
							{el.name || "Unnamed clip"}
						</Typography>
						<Typography variant="caption" color="text.secondary">
							{formatDuration(el.length ?? 0)} · User {el.user_id}
						</Typography>
					</Box>
				</AccordionSummary>
				<AccordionDetails>
					<Stack spacing={0.5}>
						{isComposedClip(el) ? (
							<Typography variant="caption" color="text.secondary">
								Composed in the clip editor
							</Typography>
						) : (
							<>
								<Typography variant="body2">
									Channel {el.channel_id} · source offset{" "}
									{formatDuration(el.start_time)}
								</Typography>
								<Typography
									variant="caption"
									color="text.secondary"
									sx={{ overflowWrap: "anywhere" }}
								>
									{el.original_file_name || "Unknown source recording"}
								</Typography>
							</>
						)}
						<AlertDialog
							clip_id={el.clip_id}
							canDelete={canDeleteClip(
								props.guildSelected,
								props.currentUserId,
								el.user_id,
							)}
						/>
					</Stack>
				</AccordionDetails>
			</Accordion>
		);
	});
	return <div>{elements}</div>;
}

function AlertDialog(props: { clip_id: string; canDelete: boolean }) {
	const [open, setOpen] = useState(false);

	const handleClickOpen = () => {
		if (!props.canDelete) return;
		setOpen(true);
	};

	const handleClose = () => {
		setOpen(false);
	};
	const params = useParams();

	const [deleteClip] = useDeleteClipMutation();

	const handleYes = async () => {
		if (params.guild_id) {
			try {
				await deleteClip({
					guild_id: params.guild_id,
					file_name: props.clip_id,
				}).unwrap();
				setOpen(false);
			} catch (error) {
				console.error("Failed to delete clip:", error);
				setOpen(false);
			}
		} else {
			setOpen(false);
		}
	};

	return (
		<div>
			<Button
				variant="danger"
				size="sm"
				isDisabled={!props.canDelete}
				onClick={handleClickOpen}
			>
				Delete
			</Button>
			<Dialog
				open={open}
				onClose={handleClose}
				aria-labelledby="alert-dialog-title"
				aria-describedby="alert-dialog-description"
			>
				<DialogTitle id="alert-dialog-title">Confirm deletion?</DialogTitle>
				<DialogContent>
					<DialogContentText id="alert-dialog-description">
						Are you sure you want to delete the clip?
					</DialogContentText>
				</DialogContent>
				<DialogActions>
					<Button variant="secondary" onClick={handleClose}>
						No
					</Button>
					<Button variant="danger" onClick={handleYes} autoFocus>
						YEP
					</Button>
				</DialogActions>
			</Dialog>
		</div>
	);
}

function clipAbsoluteStartMs(clip: ClipData | null): number | null {
	if (!clip?.original_file_name) return null;
	const ts = Number.parseInt(clip.original_file_name.split("-")[0] ?? "", 10);
	if (!Number.isFinite(ts)) return null;
	return ts + clip.start_time * 1000;
}

export default function Clips() {
	const params = useParams();

	const guildSelected = useAppSelector((state) => state.app.guildSelected);
	const { data: authData } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});
	const guildId = params.guild_id ?? "";
	const guild =
		authData?.guilds?.find((g) => g.id === guildId) ?? guildSelected;
	const { asRoleArg } = useAsRole();
	const { data, isError, isSuccess } = useGetClipsQuery(
		{ guild_id: guildId, ...asRoleArg },
		{
			skip: !guildId,
			refetchOnMountOrArgChange: true,
		},
	);

	if (isError) {
		console.error("cannot get clip data");
	}

	if (isSuccess && data) {
		return (
			<Page className="max-w-7xl space-y-4">
				<ViewAsRoleBanner guildId={guildId} />
				<ClipsLayout
					data={data}
					params={params}
					currentUserId={authData?.user?.user_id ?? null}
					guildSelected={guild}
				/>
			</Page>
		);
	} else {
		return (
			<Page>
				<Text tone="muted">No clip data</Text>
			</Page>
		);
	}
}

function ClipsLayout(props: {
	data: ClipData[];
	params: { guild_id?: string; file_name?: string };
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const theme = useTheme();
	const isDesktop = useMediaQuery(theme.breakpoints.up("md"));
	const [drawerOpen, setDrawerOpen] = useState(false);
	const navigate = useNavigate();
	const [clipTab, setClipTab] = useState<"clips" | "combined">(() => {
		const selectedClipId = props.params.file_name
			? decodeURIComponent(props.params.file_name)
			: null;
		const selected = selectedClipId
			? props.data.find((c) => c.clip_id === selectedClipId)
			: null;
		return selected && isComposedClip(selected) ? "combined" : "clips";
	});

	useEffect(() => {
		if (!isDesktop) setDrawerOpen(false);
	}, [isDesktop]);

	const composedClips = props.data.filter(isComposedClip);
	const shownClips =
		clipTab === "combined"
			? composedClips
			: props.data.filter((clip) => !isComposedClip(clip));

	const list = (
		<>
			<Box sx={{ p: 1 }}>
				<Button
					variant="contained"
					fullWidth
					startIcon={<ContentCutIcon />}
					onClick={() =>
						navigate(
							`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.params.guild_id}/clips/editor`,
						)
					}
				>
					Clip editor
				</Button>
			</Box>
			<Tabs
				value={clipTab}
				onChange={(_event, value: "clips" | "combined") => setClipTab(value)}
				variant="fullWidth"
				sx={{ borderBottom: 1, borderColor: "divider" }}
			>
				<Tab label="Clips" value="clips" />
				<Tab
					label={`Combined${composedClips.length > 0 ? ` (${composedClips.length})` : ""}`}
					value="combined"
				/>
			</Tabs>
			{clipTab === "combined" && composedClips.length === 0 && (
				<Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
					No combined clips yet. Export a composition from the clip editor.
				</Typography>
			)}
			<SimpleAccordion
				data={shownClips}
				currentUserId={props.currentUserId}
				guildSelected={props.guildSelected}
			/>
		</>
	);

	const selectedClipId = props.params.file_name
		? decodeURIComponent(props.params.file_name)
		: null;
	const selectedClip = selectedClipId
		? props.data.find((c) => c.clip_id === selectedClipId)
		: null;
	const absoluteStartMs = clipAbsoluteStartMs(selectedClip ?? null);

	return (
		<Box
			sx={{
				display: "flex",
				flexDirection: { xs: "column", md: "row" },
				width: "100%",
				gap: 1,
			}}
		>
			{isDesktop ? (
				<Box
					sx={{
						flex: "0 0 34%",
						maxWidth: 480,
						width: "100%",
						overflow: "auto",
						p: 1,
					}}
				>
					{list}
				</Box>
			) : (
				<Box sx={{ p: 1 }}>
					<Button
						variant="outlined"
						fullWidth
						startIcon={<MovieIcon />}
						onClick={() => setDrawerOpen(true)}
					>
						Browse clips
					</Button>
					<Typography
						variant="body2"
						color="text.secondary"
						sx={{ mt: 1, px: 0.5, wordBreak: "break-word" }}
					>
						{selectedClip
							? `Current: ${selectedClip.name}`
							: "No clip selected"}
					</Typography>
					<Drawer
						anchor="left"
						open={drawerOpen}
						onClose={() => setDrawerOpen(false)}
					>
						<Box sx={{ width: 320 }}>{list}</Box>
					</Drawer>
				</Box>
			)}
			<Box sx={{ flex: 1, minWidth: 0 }}>
				{selectedClip && (
					<ClipPlayer
						key={selectedClip.clip_id}
						clip={selectedClip}
						absoluteStartMs={absoluteStartMs}
						canRename={canDeleteClip(
							props.guildSelected,
							props.currentUserId,
							selectedClip.user_id,
						)}
					/>
				)}
			</Box>
		</Box>
	);
}
