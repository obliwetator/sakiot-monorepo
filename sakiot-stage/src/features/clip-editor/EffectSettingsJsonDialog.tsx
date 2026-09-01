import { useEffect, useState } from "react";
import { BaseDialog } from "../../shared/BaseDialog";
import {
	Button,
	LegacyTextField as TextField,
	Typography,
} from "../../shared/ui";
import { parseEffectSettingsJson } from "./effectSettingsJson";
import { DEFAULT_EFFECTS, resizeSelectedSegments } from "./model";
import type { UseClipEditorReturn } from "./useClipEditor";

export function EffectSettingsJsonDialog(props: {
	open: boolean;
	onClose: () => void;
	editor: UseClipEditorReturn;
}) {
	const [json, setJson] = useState("");
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!props.open) return;
		setJson(
			JSON.stringify(
				props.editor.selectedSegment?.effects ?? DEFAULT_EFFECTS,
				null,
				2,
			),
		);
		setError(null);
	}, [props.editor.selectedSegment, props.open]);

	const apply = () => {
		const ids = props.editor.selectedSegmentIds;
		if (ids.length === 0) {
			setError("Select at least one timeline segment first.");
			return;
		}
		const result = parseEffectSettingsJson(json);
		if (!result.ok) {
			setError(result.error);
			return;
		}
		props.editor.apply((edit) =>
			resizeSelectedSegments(edit, ids, (_id, effects) => ({
				...effects,
				...result.patch,
			})),
		);
		props.onClose();
	};

	return (
		<BaseDialog
			open={props.open}
			onClose={props.onClose}
			title="Effect settings JSON"
			error={error ?? undefined}
			actions={
				<>
					<Button onClick={props.onClose}>Cancel</Button>
					<Button variant="contained" onClick={apply}>
						Apply to selected
					</Button>
				</>
			}
		>
			<Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
				Paste a complete or partial camelCase effect object. Values apply to all
				selected segments; omitted settings stay unchanged.
			</Typography>
			<TextField
				autoFocus
				fullWidth
				multiline
				minRows={16}
				maxRows={24}
				label="Segment effects"
				value={json}
				onChange={(event) => {
					setJson(event.currentTarget.value);
					setError(null);
				}}
				inputProps={{
					spellCheck: false,
					style: { fontFamily: "monospace", fontSize: "0.8rem" },
				}}
			/>
		</BaseDialog>
	);
}
