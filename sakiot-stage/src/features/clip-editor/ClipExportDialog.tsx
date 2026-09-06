import { BaseDialog } from "../../shared/BaseDialog";
import {
	Button,
	ProgressBar,
	Radio,
	RadioGroup,
	TextField,
} from "../../shared/ui";

export function ClipExportDialog(props: {
	open: boolean;
	name: string;
	setName: (name: string) => void;
	error: string | null;
	isStarting: boolean;
	isRendering: boolean;
	progress: number;
	stage: string;
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
					<Button onPress={props.onClose}>
						{busy ? "Close" : props.done ? "Done" : "Cancel"}
					</Button>
					{!props.done && (
						<Button
							variant="primary"
							isDisabled={busy || props.segmentCount === 0}
							onPress={props.onStart}
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
			<div className="w-110 min-h-57.5 flex flex-col">
				{props.overwriteAvailable && (
					<fieldset disabled={busy} className="relative flex min-w-0 mb-4">
						<RadioGroup
							aria-label="Export destination"
							value={props.overwrite ? "overwrite" : "new"}
							onChange={(value) => props.setOverwrite(value === "overwrite")}
						>
							<Radio value={"new"}>Save as new clip</Radio>
							<Radio value={"overwrite"}>Overwrite this combined clip</Radio>
						</RadioGroup>
					</fieldset>
				)}
				<TextField
					label="Clip name"
					value={props.name}
					className="mb-4"
					isDisabled={busy}
					description={
						props.overwrite
							? "Leave empty to keep the current clip's name."
							: undefined
					}
					onChange={(value) => props.setName(value)}
				/>
				{props.done ? (
					<p className="text-sm">
						{props.overwrite
							? "Updated — the combined clip now reflects this version."
							: "Exported — the new clip is now in the bin."}
					</p>
				) : props.isRendering ? (
					<div>
						<p className="text-sm mb-2">
							{props.stage === "queued"
								? "Waiting for an available export slot…"
								: props.stage === "retrying"
									? "Retrying the export after an interruption…"
									: props.stage === "preparing"
										? "Preparing source audio…"
										: props.stage === "publishing"
											? "Saving the finished clip…"
											: "Rendering audio…"}
						</p>
						<ProgressBar value={props.progress} />
						<p className="text-muted text-xs mt-2">
							You can close this dialog. The export continues on the server.
						</p>
						<span className="text-muted text-xs leading-5 tabular-nums">
							{props.progress}%
						</span>
					</div>
				) : (
					<p className="text-muted text-sm">
						{props.overwrite
							? `Renders ${props.segmentCount} segment${props.segmentCount === 1 ? "" : "s"} and replaces the combined clip with this version.`
							: `Renders ${props.segmentCount} segment${props.segmentCount === 1 ? "" : "s"} into a single new clip.`}
					</p>
				)}
			</div>
		</BaseDialog>
	);
}
