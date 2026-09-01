import { BaseDialog } from "../../shared/BaseDialog";
import {
	Box,
	Button,
	FormControl,
	FormControlLabel,
	LinearProgress,
	Radio,
	RadioGroup,
	LegacyTextField as TextField,
	Typography,
} from "../../shared/ui";

export function ClipExportDialog(props: {
	open: boolean;
	name: string;
	setName: (name: string) => void;
	error: string | null;
	isStarting: boolean;
	isRendering: boolean;
	progress: number;
	done: boolean;
	segmentCount: number;
	overwriteAvailable: boolean;
	overwrite: boolean;
	setOverwrite: (overwrite: boolean) => void;
	onStart: () => void;
	onClose: () => void;
}) {
	const busy = props.isStarting || props.isRendering;
	return (
		<BaseDialog
			open={props.open}
			onClose={props.onClose}
			title="Export composition"
			error={props.error ?? undefined}
			busy={busy}
			actions={
				<>
					<Button onClick={props.onClose} disabled={busy}>
						{props.done ? "Done" : "Cancel"}
					</Button>
					{!props.done && (
						<Button
							variant="contained"
							disabled={busy || props.segmentCount === 0}
							onClick={props.onStart}
						>
							{props.overwrite ? "Overwrite" : "Render"}
						</Button>
					)}
				</>
			}
		>
			{/* Fixed footprint: the overwrite/new toggle and helper text change
			    the content height, so pin the box size to keep the dialog from
			    resizing while the choice changes. */}
			<Box
				sx={{
					width: 440,
					minHeight: 230,
					display: "flex",
					flexDirection: "column",
				}}
			>
				{props.overwriteAvailable && (
					<FormControl component="fieldset" disabled={busy} sx={{ mb: 2 }}>
						<RadioGroup
							value={props.overwrite ? "overwrite" : "new"}
							onChange={(event) =>
								props.setOverwrite(event.currentTarget.value === "overwrite")
							}
						>
							<FormControlLabel
								value="new"
								control={<Radio size="small" />}
								label="Save as new clip"
							/>
							<FormControlLabel
								value="overwrite"
								control={<Radio size="small" />}
								label="Overwrite this combined clip"
							/>
						</RadioGroup>
					</FormControl>
				)}
				<TextField
					size="small"
					fullWidth
					label="Clip name"
					value={props.name}
					disabled={busy}
					onChange={(event) => props.setName(event.currentTarget.value)}
					helperText={
						props.overwrite
							? "Leave empty to keep the current clip's name."
							: undefined
					}
					sx={{ mb: 2 }}
				/>
				{props.done ? (
					<Typography variant="body2">
						{props.overwrite
							? "Updated — the combined clip now reflects this version."
							: "Exported — the new clip is now in the bin."}
					</Typography>
				) : props.isRendering ? (
					<Box>
						<Typography variant="body2" sx={{ mb: 1 }}>
							Rendering {props.segmentCount} segment
							{props.segmentCount === 1 ? "" : "s"} on the server…
						</Typography>
						<LinearProgress variant="determinate" value={props.progress} />
						<Typography
							variant="caption"
							color="text.secondary"
							sx={{ fontVariantNumeric: "tabular-nums" }}
						>
							{props.progress}%
						</Typography>
					</Box>
				) : (
					<Typography variant="body2" color="text.secondary">
						{props.overwrite
							? `Renders ${props.segmentCount} segment${props.segmentCount === 1 ? "" : "s"} and replaces the combined clip with this version.`
							: `Renders ${props.segmentCount} segment${props.segmentCount === 1 ? "" : "s"} into a single new clip.`}
					</Typography>
				)}
			</Box>
		</BaseDialog>
	);
}
