import SearchIcon from "@mui/icons-material/Search";
import Box from "@mui/material/Box";
import CircularProgress from "@mui/material/CircularProgress";
import InputAdornment from "@mui/material/InputAdornment";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemText from "@mui/material/ListItemText";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import { useState } from "react";
import type { ClipData } from "../../app/apiSlice";
import { formatDuration } from "../../utils/formatTime";

export function ClipBin(props: {
	clips: ClipData[];
	loadingClips: ReadonlyMap<string, boolean>;
	onAdd: (clip: ClipData) => void;
	onDeselect: () => void;
}) {
	const [search, setSearch] = useState("");
	const query = search.trim().toLowerCase();
	const filtered = query
		? props.clips.filter((clip) =>
				(clip.name ?? clip.clip_id).toLowerCase().includes(query),
			)
		: props.clips;

	return (
		<Box
			onPointerDown={(event) => {
				if (event.button === 0) props.onDeselect();
			}}
			sx={{
				width: 280,
				flex: "0 0 auto",
				display: "flex",
				flexDirection: "column",
				minHeight: 0,
				borderRight: 1,
				borderColor: "divider",
			}}
		>
			<Box sx={{ p: 1.5, pb: 1 }}>
				<TextField
					size="small"
					fullWidth
					placeholder="Search clips"
					value={search}
					onChange={(event) => setSearch(event.currentTarget.value)}
					slotProps={{
						input: {
							startAdornment: (
								<InputAdornment position="start">
									<SearchIcon fontSize="small" />
								</InputAdornment>
							),
						},
					}}
				/>
				<Typography variant="caption" color="text.secondary" sx={{ px: 0.5 }}>
					Drag onto a track, or double-click to append
				</Typography>
			</Box>
			<Box
				sx={{
					flex: 1,
					minHeight: 0,
					overflowY: "auto",
					overflowX: "hidden",
				}}
			>
				<List dense disablePadding>
					{filtered.map((clip) => (
						<ClipBinItem
							key={clip.clip_id}
							clip={clip}
							loading={props.loadingClips.get(clip.clip_id) === true}
							onAdd={props.onAdd}
						/>
					))}
					{filtered.length === 0 && (
						<Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
							No clips match.
						</Typography>
					)}
				</List>
			</Box>
		</Box>
	);
}

function ClipBinItem(props: {
	clip: ClipData;
	loading: boolean;
	onAdd: (clip: ClipData) => void;
}) {
	return (
		<ListItemButton
			draggable
			onDragStart={(event) => {
				event.dataTransfer.setData("text/plain", props.clip.clip_id);
				event.dataTransfer.effectAllowed = "copy";
			}}
			onDoubleClick={() => props.onAdd(props.clip)}
			sx={{ px: 1.5 }}
		>
			<ListItemText
				primary={props.clip.name || "Unnamed clip"}
				primaryTypographyProps={{ noWrap: true }}
				secondary={formatDuration(props.clip.length ?? 0)}
				secondaryTypographyProps={{ variant: "caption" }}
			/>
			{props.loading && <CircularProgress size={14} />}
		</ListItemButton>
	);
}
