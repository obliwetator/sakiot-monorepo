import { type ReactNode, useState } from "react";
import { problemFromError, problemFromQueryError } from "../app/apiError";
import { useUndoErasureMutation } from "../app/apiSlice";
import { BaseDialog } from "./BaseDialog";
import { Button, Notice } from "./ui";

/** How long a speaker can undo an erasure: the server's `GRACE_DAYS`. */
export const ERASURE_GRACE_DAYS = 30;

/** The day an erasure becomes permanent. */
export function erasureDay(purgeAtMs: number): string {
	return new Date(purgeAtMs).toLocaleDateString(undefined, {
		dateStyle: "long",
	});
}

/**
 * Erases something of the speaker's own voice once they confirm: it says
 * what everyone else stops seeing now, and that after the grace period
 * nothing can bring it back.
 */
export function EraseButton(props: {
	label: string;
	title: string;
	/** What goes with it, and what the speaker keeps meanwhile. */
	children: ReactNode;
	busy: boolean;
	erase: () => Promise<unknown>;
}) {
	const [open, setOpen] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const close = () => {
		setOpen(false);
		setError(null);
	};
	return (
		<>
			<Button variant="danger" onPress={() => setOpen(true)}>
				{props.label}
			</Button>
			<BaseDialog
				open={open}
				onClose={close}
				title={props.title}
				busy={props.busy}
				error={error ?? undefined}
				confirmLabel={error ? "Try again" : "Erase"}
				confirmVariant="danger"
				onConfirm={() => {
					setError(null);
					props.erase().then(close, (failure) => {
						// Keep the dialog open so the failure is visible and can be retried.
						setError(
							`Nothing was erased. ${problemFromError(failure).message}`,
						);
					});
				}}
				autoFocusCancel
			>
				{props.children}
				<p className="text-sm leading-6 text-slate-200">
					You can undo this for {ERASURE_GRACE_DAYS} days. After that it is
					deleted for good, and nobody can bring it back.
				</p>
			</BaseDialog>
		</>
	);
}

/** Undoes the speaker's erasure while its grace period lasts. */
export function UndoErasureButton(props: {
	guildId: string;
	erasureId: string;
}) {
	const [undo, state] = useUndoErasureMutation();
	return (
		<div className="space-y-2">
			<Button
				variant="outline"
				isDisabled={state.isLoading}
				onPress={() => {
					void undo({ guild_id: props.guildId, erasure_id: props.erasureId })
						.unwrap()
						// The mutation state renders the failure below.
						.catch(() => {});
				}}
			>
				Undo
			</Button>
			{state.isError && (
				<Notice tone="error" announce="alert">
					Could not undo the erasure.{" "}
					{problemFromQueryError(state.error).message}
				</Notice>
			)}
		</div>
	);
}
