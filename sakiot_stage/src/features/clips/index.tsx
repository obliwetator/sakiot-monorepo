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
import { AudioInterface } from "../audio-dashboard/AudioInterface";

function SimpleAccordion(props: {
	data: ClipData[];
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const navigate = useNavigate();
	const [expanded, setExpanded] = useState<string | false>(false);

	const handleClickAccordion = (guild_id: string, clip_id: string) => {
		console.log(
			"here",
			`${PATH_PREFIX_FOR_LOGGED_USERS}/${guild_id}/clips/${encodeURIComponent(clip_id)}`,
		);
		if (
			location.pathname ===
			`${PATH_PREFIX_FOR_LOGGED_USERS}/${guild_id}/clips/${encodeURIComponent(clip_id)}`
		) {
			// do nothing
		} else {
			navigate(
				`${PATH_PREFIX_FOR_LOGGED_USERS}/${guild_id}/clips/${encodeURIComponent(clip_id)}`,
			);
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
					<Typography>CLIP_NAME: {el.name}</Typography>
				</AccordionSummary>
				<AccordionDetails>
					<Typography>Start time: {formatDuration(el.start_time)}</Typography>
					<Typography>BY(not working): {el.user_id}</Typography>
					<Typography>
						length:{" "}
						{typeof el.length === "number" ? el.length.toFixed(2) : "unknown"}
					</Typography>
					<Typography>size: {el.size}</Typography>
					<Typography>OG file: {el.original_file_name}</Typography>
					<div>
						<AlertDialog
							clip_id={el.clip_id}
							canDelete={canDeleteClip(
								props.guildSelected,
								props.currentUserId,
								el.user_id,
							)}
						/>
					</div>
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
	const location = useLocation();

	const guildSelected = useAppSelector((state) => state.app.guildSelected);
	const { data: authData } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});
	const userGuilds = authData?.guilds || null;

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
				location={location}
				userGuilds={userGuilds}
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
	location: ReturnType<typeof useLocation>;
	userGuilds: UserGuilds[] | null;
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
						flex: "0 0 40%",
						maxWidth: 480,
						width: "100%",
						overflow: "auto",
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
				{props.params.file_name && (
					<AudioInterface
						key={props.location.pathname}
						isClip={true}
						userGuilds={props.userGuilds}
						isSilence={false}
						absoluteStartMs={absoluteStartMs}
					/>
				)}
			</Box>
		</Box>
	);
}
