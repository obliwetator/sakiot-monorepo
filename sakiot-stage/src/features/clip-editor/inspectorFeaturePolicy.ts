export const MULTI_SELECTION_DISABLED_REASON =
	"Unavailable while multiple segments are selected.";

export interface InspectorFeaturePolicy {
	/**
	 * Add this only after the feature's multi-selection behavior has been
	 * implemented and tested. Omission is the safe, single-selection default.
	 */
	multiSelection?: "handled";
}

export const INSPECTOR_FEATURE_POLICIES = {
	volume: { multiSelection: "handled" },
	pitch: { multiSelection: "handled" },
	speed: { multiSelection: "handled" },
	bass: { multiSelection: "handled" },
	treble: { multiSelection: "handled" },
	split: {},
	reverse: { multiSelection: "handled" },
	delete: { multiSelection: "handled" },
} as const satisfies Record<string, InspectorFeaturePolicy>;

export type InspectorFeatureId = keyof typeof INSPECTOR_FEATURE_POLICIES;

/**
 * The policy itself defaults to unhandled so new inspector features are safe
 * in multi-selection mode until a developer explicitly opts them in.
 */
export function isMultiSelectionDisabled(
	selectionCount: number,
	policy: InspectorFeaturePolicy = {},
): boolean {
	return selectionCount > 1 && policy.multiSelection !== "handled";
}

export function isInspectorFeatureDisabled(
	feature: InspectorFeatureId,
	selectionCount: number,
): boolean {
	const policy: InspectorFeaturePolicy = INSPECTOR_FEATURE_POLICIES[feature];
	return isMultiSelectionDisabled(selectionCount, policy);
}
