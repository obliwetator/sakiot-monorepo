function targetElement(target: EventTarget | null): HTMLElement | null {
	return target instanceof HTMLElement ? target : null;
}

/** Text-entry controls retain ordinary typing instead of triggering playback. */
export function playbackShortcutTargetAcceptsText(
	target: EventTarget | null,
): boolean {
	const element = targetElement(target);
	if (!element) return false;
	return Boolean(
		element.closest(
			'input:not([type="range"]):not([type="button"]):not([type="checkbox"]):not([type="radio"]):not([type="reset"]):not([type="submit"]), textarea, select, [contenteditable]:not([contenteditable="false"])',
		),
	);
}

/** Controls with native left/right behavior keep ownership of arrow presses. */
export function playbackShortcutTargetOwnsArrows(
	target: EventTarget | null,
): boolean {
	const element = targetElement(target);
	if (!element) return false;
	return (
		playbackShortcutTargetAcceptsText(target) ||
		Boolean(
			element.closest(
				'input[type="range"], [role="slider"], [role="tab"], [role="tablist"], [role="menuitem"], [role="combobox"], [role="listbox"], [role="option"]',
			),
		)
	);
}
