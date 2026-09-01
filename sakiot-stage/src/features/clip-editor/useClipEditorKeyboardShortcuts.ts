import { useEffect, useRef } from "react";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "../audio-dashboard/playbackShortcuts";
import { isEffectSettingsJsonShortcut } from "./effectSettingsJson";
import { isInspectorFeatureDisabled } from "./inspectorFeaturePolicy";
import type { UseClipEditorReturn } from "./useClipEditor";

export function useClipEditorKeyboardShortcuts(
	editor: UseClipEditorReturn,
	openEffectSettingsJson: () => void,
	openOptions: () => void,
) {
	const editorRef = useRef(editor);
	const openEffectSettingsJsonRef = useRef(openEffectSettingsJson);
	const openOptionsRef = useRef(openOptions);
	useEffect(() => {
		editorRef.current = editor;
		openEffectSettingsJsonRef.current = openEffectSettingsJson;
		openOptionsRef.current = openOptions;
	}, [editor, openEffectSettingsJson, openOptions]);

	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			const current = editorRef.current;
			if (isEffectSettingsJsonShortcut(event)) {
				event.preventDefault();
				openEffectSettingsJsonRef.current();
				return;
			}
			if (playbackShortcutTargetAcceptsText(event.target)) return;
			const modifier = event.ctrlKey || event.metaKey;
			if (modifier && event.key.toLowerCase() === "z") {
				event.preventDefault();
				if (event.shiftKey) current.redo();
				else current.undo();
				return;
			}
			if (modifier && event.key.toLowerCase() === "y") {
				event.preventDefault();
				current.redo();
				return;
			}
			if (modifier && event.key.toLowerCase() === "c") {
				if (current.selectedSegment) {
					event.preventDefault();
					current.copy();
				}
				return;
			}
			if (modifier && event.key.toLowerCase() === "x") {
				if (current.selectedSegment) {
					event.preventDefault();
					current.cut();
				}
				return;
			}
			if (modifier && event.key.toLowerCase() === "v") {
				event.preventDefault();
				current.paste();
				return;
			}
			if (modifier && event.key.toLowerCase() === "a") {
				event.preventDefault();
				current.selectMany(current.edit.segments.map((segment) => segment.id));
				return;
			}
			if (modifier && event.key === ",") {
				event.preventDefault();
				openOptionsRef.current();
				return;
			}
			if (modifier) return;
			if (event.key === "Escape") {
				if (current.selectedSegments.length > 0) {
					event.preventDefault();
					current.select(null);
				}
				return;
			}
			if (event.key === " " || event.code === "Space") {
				if (event.repeat) return;
				event.preventDefault();
				current.togglePlay();
				return;
			}
			const segment = current.selectedSegment;
			if (event.key === "Delete" || event.key === "Backspace") {
				if (segment) {
					if (
						isInspectorFeatureDisabled(
							"delete",
							current.selectedSegments.length,
						)
					)
						return;
					event.preventDefault();
					current.removeSelected();
				}
				return;
			}
			if (segment) {
				if (event.key === "s" || event.key === "S") {
					if (
						isInspectorFeatureDisabled("split", current.selectedSegments.length)
					)
						return;
					event.preventDefault();
					current.splitSelectedAtPlayhead();
					return;
				}
				if (event.key === "r" || event.key === "R") {
					if (
						isInspectorFeatureDisabled(
							"reverse",
							current.selectedSegments.length,
						)
					)
						return;
					event.preventDefault();
					current.toggleReverse();
					return;
				}
				if (event.key === "m" || event.key === "M") {
					event.preventDefault();
					current.mergeSelected();
					return;
				}
			}
			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			const distance = event.shiftKey ? 1 : 0.1;
			event.preventDefault();
			current.setPosition(
				Math.max(
					0,
					current.positionSec +
						(event.key === "ArrowRight" ? distance : -distance),
				),
			);
		};
		window.addEventListener("keydown", handleShortcut);
		return () => window.removeEventListener("keydown", handleShortcut);
	}, []);
}
