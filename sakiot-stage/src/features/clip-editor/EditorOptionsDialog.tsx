import { BaseDialog } from "../../shared/BaseDialog";
import { Button, FormControlLabel, Switch, Typography } from "../../shared/ui";
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
			<FormControlLabel
				control={
					<Switch
						size="small"
						checked={props.options.audacityStyleInteraction}
						onChange={(_event, checked) =>
							props.onChange({
								...props.options,
								audacityStyleInteraction: checked,
							})
						}
					/>
				}
				label={
					<Typography variant="body2">
						Audacity-style segment interaction
					</Typography>
				}
			/>
			<Typography variant="caption" color="text.secondary" display="block">
				Only the narrow bar at the top of a segment selects or moves it.
				Clicking elsewhere starts marquee selection.
			</Typography>
			<FormControlLabel
				control={
					<Switch
						size="small"
						checked={props.options.copyAllSelected}
						onChange={(_event, checked) =>
							props.onChange({ ...props.options, copyAllSelected: checked })
						}
					/>
				}
				label={
					<Typography variant="body2">Copy all selected elements</Typography>
				}
			/>
			<Typography variant="caption" color="text.secondary" display="block">
				When enabled, Ctrl/Cmd+C copies every selected element. When disabled,
				it copies only the earliest selected element in the timeline.
			</Typography>
		</BaseDialog>
	);
}
