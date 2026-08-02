export interface ShortcutKeyState {
	key: string;
	repeat: boolean;
	ctrlKey: boolean;
	metaKey: boolean;
	altKey: boolean;
}

/** Reset is a plain R press; browser/OS modified shortcuts retain priority. */
export function isResetClipSelectionShortcut(event: ShortcutKeyState): boolean {
	return (
		(event.key === "r" || event.key === "R") &&
		!event.repeat &&
		!event.ctrlKey &&
		!event.metaKey &&
		!event.altKey
	);
}
