export interface EditorOptions {
	/** Marquee click-drag selection covers every track the rectangle spans
	 * instead of only the track the drag started on. */
	marqueeMultiTrack: boolean;
	/** Only the narrow bar at the top of a segment selects or moves it. */
	audacityStyleInteraction: boolean;
	/** Ctrl/Cmd+C copies every selected visual element instead of one. */
	copyAllSelected: boolean;
}

export const DEFAULT_EDITOR_OPTIONS: EditorOptions = {
	marqueeMultiTrack: false,
	audacityStyleInteraction: false,
	copyAllSelected: true,
};

const STORAGE_KEY = "sakiot:clip-editor:options";

export interface StorageLike {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

const defaultStorage = (): StorageLike | null =>
	typeof globalThis.localStorage === "undefined"
		? null
		: globalThis.localStorage;

/**
 * Load the saved options, falling back to defaults per key so unknown or
 * malformed entries (older or newer versions of the app) never break the
 * editor.
 */
export function loadEditorOptions(
	storage: StorageLike | null = defaultStorage(),
): EditorOptions {
	const options = { ...DEFAULT_EDITOR_OPTIONS };
	if (!storage) return options;
	const raw = storage.getItem(STORAGE_KEY);
	if (!raw) return options;
	try {
		const parsed = JSON.parse(raw) as Record<string, unknown>;
		if (typeof parsed.marqueeMultiTrack === "boolean") {
			options.marqueeMultiTrack = parsed.marqueeMultiTrack;
		}
		if (typeof parsed.audacityStyleInteraction === "boolean") {
			options.audacityStyleInteraction = parsed.audacityStyleInteraction;
		}
		if (typeof parsed.copyAllSelected === "boolean") {
			options.copyAllSelected = parsed.copyAllSelected;
		}
		return options;
	} catch {
		return options;
	}
}

/** Persist the options; failures (quota, privacy mode) are swallowed. */
export function saveEditorOptions(
	options: EditorOptions,
	storage: StorageLike | null = defaultStorage(),
): void {
	if (!storage) return;
	try {
		storage.setItem(STORAGE_KEY, JSON.stringify(options));
	} catch {
		// Storage is best-effort; ignore quota and availability errors.
	}
}
