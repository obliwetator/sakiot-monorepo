import { useEffect, useState } from "react";
import { BaseDialog } from "../../shared/BaseDialog";
import { Button, TextArea } from "../../shared/ui";
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
					<Button onPress={props.onClose}>Cancel</Button>
					<Button variant="primary" onPress={apply}>
						Apply to selected
					</Button>
				</>
			}
		>
			<p className="text-muted text-sm mb-3">
				Paste a complete or partial camelCase effect object. Values apply to all
				selected segments; omitted settings stay unchanged.
			</p>
			<TextArea
				autoFocus
				label="Segment effects"
				value={json}
				rows={16}
				onChange={(value) => {
					setJson(value);
					setError(null);
				}}
				spellCheck={false}
				inputStyle={{ fontFamily: "monospace", fontSize: "0.8rem" }}
			/>
		</BaseDialog>
	);
}
