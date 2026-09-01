import { useEffect } from "react";
import { useBlocker } from "react-router-dom";
import { BaseDialog } from "../../shared/BaseDialog";
import { Button, DialogContentText } from "../../shared/ui";

/**
 * Warns before the clip editor is left with unsaved work. In-app navigation
 * is blocked through the router (data routers only); closing or reloading the
 * tab triggers the native browser dialog. `dirty` is derived from the edit
 * history: any undoable or redoable step means the page has work on it.
 */
export function useUnsavedChangesGuard(dirty: boolean) {
	const blocker = useBlocker(dirty);
	// The Blocker union only exposes reset/proceed on the "blocked" member.
	const blockedBlocker = blocker?.state === "blocked" ? blocker : null;

	useEffect(() => {
		if (!dirty) return;
		const onBeforeUnload = (event: BeforeUnloadEvent) => {
			event.preventDefault();
		};
		window.addEventListener("beforeunload", onBeforeUnload);
		return () => window.removeEventListener("beforeunload", onBeforeUnload);
	}, [dirty]);

	const dialog = (
		<BaseDialog
			open={blockedBlocker !== null}
			onClose={() => blockedBlocker?.reset()}
			title="Discard clip editor work?"
			actions={
				<>
					<Button onClick={() => blockedBlocker?.reset()} autoFocus>
						Stay
					</Button>
					<Button
						variant="contained"
						color="error"
						onClick={() => blockedBlocker?.proceed()}
					>
						Discard and leave
					</Button>
				</>
			}
		>
			<DialogContentText>
				The clip editor still has unsaved changes. Leaving this page will
				discard them.
			</DialogContentText>
		</BaseDialog>
	);

	return { dialog, dirty };
}
