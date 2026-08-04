import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import MovieIcon from "@mui/icons-material/Movie";
import Accordion from "@mui/material/Accordion";
import AccordionDetails from "@mui/material/AccordionDetails";
import AccordionSummary from "@mui/material/AccordionSummary";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Dialog from "@mui/material/Dialog";
import DialogActions from "@mui/material/DialogActions";
import DialogContent from "@mui/material/DialogContent";
import DialogContentText from "@mui/material/DialogContentText";
import DialogTitle from "@mui/material/DialogTitle";
import Drawer from "@mui/material/Drawer";
import Stack from "@mui/material/Stack";
import { useTheme } from "@mui/material/styles";
import Typography from "@mui/material/Typography";
import useMediaQuery from "@mui/material/useMediaQuery";
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
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../../Constants";
import { canDeleteClip } from "../../shared/permissions";
import { formatDuration } from "../../utils/formatTime";
import { ClipPlayer } from "./ClipPlayer";

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
				variant="contained"
				color="error"
				disabled={!props.canDelete}
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
				<DialogTitle id="alert-dialog-title">{"Confirm deletion?"}</DialogTitle>
				<DialogContent>
					<DialogContentText id="alert-dialog-description">
						Are you sure you want to delete the clip?
					</DialogContentText>
				</DialogContent>
				<DialogActions>
					<Button onClick={handleClose}>No</Button>
					<Button onClick={handleYes} autoFocus>
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
	const { data, isError, isSuccess } = useGetClipsQuery(
		guildSelected?.id || "",
		{
			skip: !guildSelected?.id,
			refetchOnMountOrArgChange: true,
		},
	);

	if (isError) {
		console.error("cannot get clip data");
	}

	if (isSuccess && data) {
		return (
			<ClipsLayout
				data={data}
				params={params}
				currentUserId={authData?.user?.user_id ?? null}
				guildSelected={guildSelected}
			/>
		);
	} else {
		return <div>No clip data</div>;
	}
}

function ClipsLayout(props: {
	data: ClipData[];
	params: { file_name?: string };
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const theme = useTheme();
	const isDesktop = useMediaQuery(theme.breakpoints.up("md"));
	const [drawerOpen, setDrawerOpen] = useState(false);

	useEffect(() => {
		if (!isDesktop) setDrawerOpen(false);
	}, [isDesktop]);

	const list = (
		<SimpleAccordion
			data={props.data}
			currentUserId={props.currentUserId}
			guildSelected={props.guildSelected}
		/>
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
