import { BaseDialog } from "../../shared/BaseDialog";
import { Button, Switch } from "../../shared/ui";
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
				<Button variant="primary" onPress={props.onClose}>
					Done
				</Button>
			}
		>
			<Switch
				isSelected={props.options.marqueeMultiTrack}
				onChange={(checked) =>
					props.onChange({ ...props.options, marqueeMultiTrack: checked })
				}
			>
				<p className={"text-sm"}>Marquee selects across tracks</p>
			</Switch>
			<span className="text-muted block text-xs leading-5">
				When dragging a selection box, select every segment the rectangle
				touches on any track instead of only the track the drag started on.
			</span>
			<Switch
				isSelected={props.options.audacityStyleInteraction}
				onChange={(checked) =>
					props.onChange({
						...props.options,
						audacityStyleInteraction: checked,
					})
				}
			>
				<p className={"text-sm"}>Audacity-style segment interaction</p>
			</Switch>
			<span className="text-muted block text-xs leading-5">
				Only the narrow bar at the top of a segment selects or moves it.
				Clicking elsewhere starts marquee selection.
			</span>
			<Switch
				isSelected={props.options.copyAllSelected}
				onChange={(checked) =>
					props.onChange({ ...props.options, copyAllSelected: checked })
				}
			>
				<p className={"text-sm"}>Copy all selected elements</p>
			</Switch>
			<span className="text-muted block text-xs leading-5">
				When enabled, Ctrl/Cmd+C copies every selected element. When disabled,
				it copies only the earliest selected element in the timeline.
			</span>
		</BaseDialog>
	);
}
