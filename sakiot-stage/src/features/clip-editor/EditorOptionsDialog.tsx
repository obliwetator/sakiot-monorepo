import Button from "@mui/material/Button";
import FormControlLabel from "@mui/material/FormControlLabel";
import Switch from "@mui/material/Switch";
import Typography from "@mui/material/Typography";
import { BaseDialog } from "../../shared/BaseDialog";
import type { EditorOptions } from "./editorOptions";

/**
 * Per-user editor preferences, saved in the browser. Each change applies
 * immediately and persists, so the options survive reloads without any server
 * involvement.
 */
export function EditorOptionsDialog(props: {
	open: boolean;
	onClose: () => void;
	options: EditorOptions;
	onChange: (options: EditorOptions) => void;
}) {
	return (
		<BaseDialog
			open={props.open}
			onClose={props.onClose}
			title="Editor options"
			actions={
				<Button variant="contained" onClick={props.onClose}>
					Done
				</Button>
			}
		>
			<FormControlLabel
				control={
					<Switch
						size="small"
						checked={props.options.marqueeMultiTrack}
						onChange={(_event, checked) =>
							props.onChange({ ...props.options, marqueeMultiTrack: checked })
						}
					/>
				}
				label={
					<Typography variant="body2">Marquee selects across tracks</Typography>
				}
			/>
			<Typography variant="caption" color="text.secondary" display="block">
				When dragging a selection box, select every segment the rectangle
				touches on any track instead of only the track the drag started on.
			</Typography>
		</BaseDialog>
	);
}
