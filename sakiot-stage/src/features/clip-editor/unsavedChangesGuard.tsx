import { useEffect } from "react";
import { useBlocker } from "react-router-dom";
import { BaseDialog } from "../../shared/BaseDialog";
import { Button } from "../../shared/ui";
import type { DraftStatus } from "./draftPersistence";

/**
 * Warns before the clip editor is left while its draft is not saved on this
 * device. Leaving first writes the draft synchronously, so saved work (and
 * work whose save simply had not run yet) leaves without a dialog. Only a
 * failed save or a conflict with another tab blocks. In-app navigation goes
 * through the router (data routers only); closing or reloading the tab
 * triggers the native browser dialog.
 */
export function useUnsavedChangesGuard(draft: {
	status: DraftStatus;
	persisted: boolean;
	flush: () => boolean;
	download: () => void;
}) {
	const { persisted, flush } = draft;
	const blocker = useBlocker(
		({ currentLocation, nextLocation }) =>
			(currentLocation.pathname !== nextLocation.pathname ||
				currentLocation.search !== nextLocation.search) &&
			!flush(),
	);
	// The Blocker union only exposes reset/proceed on the "blocked" member.
	const blockedBlocker = blocker.state === "blocked" ? blocker : null;

	useEffect(() => {
		if (persisted) return;
		const onBeforeUnload = (event: BeforeUnloadEvent) => {
			if (flush()) return;
			event.preventDefault();
		};
		window.addEventListener("beforeunload", onBeforeUnload);
		return () => window.removeEventListener("beforeunload", onBeforeUnload);
	}, [persisted, flush]);

	const dialog = (
		<BaseDialog
			open={blockedBlocker !== null}
			onClose={() => blockedBlocker?.reset()}
			title="Leave without saving this draft?"
			actions={
				<>
					<Button
						variant="primary"
						autoFocus
						onPress={() => blockedBlocker?.reset()}
					>
						Stay
					</Button>
					<Button variant="outline" onPress={draft.download}>
						Download
					</Button>
					<Button variant="danger" onPress={() => blockedBlocker?.proceed()}>
						Leave anyway
					</Button>
				</>
			}
		>
			<p className="text-sm leading-6 text-slate-200">
				{unsavedExplanation(draft.status)}
			</p>
		</BaseDialog>
	);

	return { dialog };
}

function unsavedExplanation(status: DraftStatus): string {
	switch (status.kind) {
		case "conflict":
			return "Another tab changed this draft, so this tab's version isn't saved. Leaving keeps the other tab's version and loses this one. Download this version to keep a copy.";
		case "damaged":
			return "The draft saved on this device can't be read, so this edit hasn't been saved over it. Leaving loses this edit. Download it to keep a copy.";
		default:
			return "This draft couldn't be saved on this device. Leaving loses the changes made since it was last saved. Download the draft to keep a copy.";
	}
}
