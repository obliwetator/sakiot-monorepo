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

export interface BinDragPayload {
	clipId: string;
	lengthSec: number;
}

/**
 * The clip being dragged, captured at dragstart. dataTransfer.getData is
 * unreliable during dragover in some browsers, so the timeline reads this
 * instead; the dataTransfer copy stays for browsers that need payload data
 * to allow the drop.
 */
export const pendingBinDrag: { payload: BinDragPayload | null } = {
	payload: null,
};

function ClipBinItem(props: {
	clip: ClipData;
	loading: boolean;
	onAdd: (clip: ClipData) => void;
}) {
	return (
		<ListItemButton
			draggable
			onDragStart={(event) => {
				pendingBinDrag.payload = {
					clipId: props.clip.clip_id,
					lengthSec: props.clip.length ?? 0,
				};
				event.dataTransfer.setData(
					"application/json",
					JSON.stringify(pendingBinDrag.payload),
				);
				event.dataTransfer.effectAllowed = "copy";
				const chip = document.createElement("div");
				chip.textContent = props.clip.name || "Clip";
				chip.style.cssText =
					"position:absolute;top:-1000px;left:-1000px;padding:4px 8px;" +
					"border-radius:6px;border:1px dashed #38bdf8;" +
					"background:rgba(15,23,42,0.9);color:#e2e8f0;" +
					"font-size:12px;white-space:nowrap;";
				document.body.appendChild(chip);
				event.dataTransfer.setDragImage(
					chip,
					chip.offsetWidth / 2,
					chip.offsetHeight + 6,
				);
				requestAnimationFrame(() => chip.remove());
			}}
			onDragEnd={() => {
				pendingBinDrag.payload = null;
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
