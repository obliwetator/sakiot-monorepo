import { useEffect, useState } from "react";
import type { ChannelMixScope } from "../../app/apiSlice";

const STORAGE_KEY = "sakiot.channel-mix.options.v1";

export interface ChannelMixOptions {
	showSourceRows: boolean;
	scope: ChannelMixScope;
}

export const DEFAULT_CHANNEL_MIX_OPTIONS: ChannelMixOptions = {
	showSourceRows: false,
	scope: "all_recordings",
};

export function isChannelMixTextEntryTarget(
	target: EventTarget | null,
): boolean {
	return (
		target instanceof HTMLInputElement ||
		target instanceof HTMLTextAreaElement ||
		target instanceof HTMLSelectElement ||
		(target instanceof HTMLElement && target.isContentEditable)
	);
}

export function readChannelMixOptions(): ChannelMixOptions {
	try {
		const parsed: unknown = JSON.parse(
			localStorage.getItem(STORAGE_KEY) ?? "null",
		);
		if (
			parsed &&
			typeof parsed === "object" &&
			"showSourceRows" in parsed &&
			typeof parsed.showSourceRows === "boolean"
		) {
			const storedScope =
				"scope" in parsed &&
				(parsed.scope === "all_recordings" ||
					parsed.scope === "selected_session")
					? parsed.scope
					: DEFAULT_CHANNEL_MIX_OPTIONS.scope;
			return { showSourceRows: parsed.showSourceRows, scope: storedScope };
		}
	} catch {
		// Use defaults when storage is unavailable or malformed.
	}
	return DEFAULT_CHANNEL_MIX_OPTIONS;
}

export function writeChannelMixOptions(options: ChannelMixOptions): void {
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify(options));
	} catch {
		// Browser storage is optional.
	}
}

export function useChannelMixPreferences() {
	const [options, setOptions] = useState<ChannelMixOptions>(() =>
		readChannelMixOptions(),
	);
	const [dialogOpen, setDialogOpen] = useState(false);

	useEffect(() => {
		writeChannelMixOptions(options);
	}, [options]);
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (
				event.key !== "," ||
				!(event.ctrlKey || event.metaKey) ||
				isChannelMixTextEntryTarget(event.target)
			) {
				return;
			}
			event.preventDefault();
			setDialogOpen(true);
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	return {
		options,
		setOptions,
		dialogOpen,
		setDialogOpen,
	};
}
