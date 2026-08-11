import { ClipRangeEditorView } from "./ClipRangeEditorView";
import type { ClipRangeEditorProps } from "./clipRangeEditorTypes";
import { useClipRangeViewport } from "./useClipRangeViewport";

export type { ClipRangeEditorProps } from "./clipRangeEditorTypes";

export function ClipRangeEditor(props: ClipRangeEditorProps) {
	const controller = useClipRangeViewport(props);
	return <ClipRangeEditorView props={props} controller={controller} />;
}
