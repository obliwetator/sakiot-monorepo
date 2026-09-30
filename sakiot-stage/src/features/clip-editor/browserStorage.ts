export interface StorageLike {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

export type StorageAccess =
	| { ok: true; storage: StorageLike }
	| { ok: false; error: unknown };

/**
 * The page's localStorage, or the reason it cannot be used. Reading the
 * `localStorage` property itself throws a SecurityError when the browser
 * blocks site data, so the access is guarded, not only the reads and writes.
 */
export function browserStorage(): StorageAccess {
	try {
		const storage = globalThis.localStorage;
		if (!storage) {
			return { ok: false, error: new Error("localStorage is unavailable") };
		}
		return { ok: true, storage };
	} catch (error) {
		return { ok: false, error };
	}
}

/** localStorage for best-effort preferences; null when the browser blocks it. */
export function optionalStorage(): StorageLike | null {
	const access = browserStorage();
	return access.ok ? access.storage : null;
}
