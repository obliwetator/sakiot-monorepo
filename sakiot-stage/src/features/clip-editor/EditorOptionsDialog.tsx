import { useState } from "react";
import { FileTrigger } from "react-aria-components/FileTrigger";
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
	/** Opens a downloaded draft; resolves to an error message, or null. */
	onOpenDraftFile: (file: File) => Promise<string | null>;
}) {
	const [fileError, setFileError] = useState<string | null>(null);
	const close = () => {
		setFileError(null);
		props.onClose();
	};
	const openDraftFile = async (file: File) => {
		const error = await props.onOpenDraftFile(file);
		if (error) {
			setFileError(error);
			return;
		}
		close();
	};
	return (
		<BaseDialog
			open={props.open}
			onClose={close}
			title="Editor options"
			closeLabel="Done"
			error={fileError ?? undefined}
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
			<div className="border-t border-ui-border pt-3">
				<p className="text-sm">Draft files</p>
				<span className="text-muted block text-xs leading-5">
					Drafts save in this browser automatically. To continue a draft you
					downloaded, open it here. It replaces the current edit, and undo
					brings the current edit back.
				</span>
				<FileTrigger
					acceptedFileTypes={["application/json", ".json"]}
					onSelect={(files) => {
						const file = files?.[0];
						if (file) void openDraftFile(file);
					}}
				>
					<Button variant="outline" size="sm" className="mt-2">
						Open draft file…
					</Button>
				</FileTrigger>
			</div>
		</BaseDialog>
	);
}
