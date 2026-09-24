interface RectLike {
	left: number;
	width: number;
}

/**
 * Where a pointer sits across a horizontal band, as a fraction clamped to the
 * band's edges. The width floor keeps a zero-width element from dividing by
 * zero while a layout is still settling.
 */
export function fractionAtClientX(
	clientX: number,
	left: number,
	width: number,
): number {
	return Math.min(1, Math.max(0, (clientX - left) / Math.max(1, width)));
}

/** `fractionAtClientX` against a measured rect. */
export function fractionInRect(clientX: number, rect: RectLike): number {
	return fractionAtClientX(clientX, rect.left, rect.width);
}

/** `fractionAtClientX` against the element an event is bound to. */
export function fractionInTarget(event: {
	clientX: number;
	currentTarget: { getBoundingClientRect(): RectLike };
}): number {
	return fractionInRect(
		event.clientX,
		event.currentTarget.getBoundingClientRect(),
	);
}
