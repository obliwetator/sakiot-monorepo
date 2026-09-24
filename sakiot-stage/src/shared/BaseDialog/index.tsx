import type { ReactNode } from "react";
import { Button, type ButtonVariant, DialogHeading, Modal } from "../ui";

export interface BaseDialogProps {
	open: boolean;
	onClose: () => void;
	title?: ReactNode;
	error?: string;
	busy?: boolean;
	children: ReactNode;
	/** Replaces the whole footer. Use it only where the buttons are unusual. */
	actions?: ReactNode;
	/** Label of the lone dismiss button, when there is nothing to confirm. */
	closeLabel?: string;
	/** Setting this turns the footer into a cancel/confirm pair. */
	confirmLabel?: ReactNode;
	onConfirm?: () => void;
	cancelLabel?: string;
	confirmVariant?: ButtonVariant;
	confirmDisabled?: boolean;
	/** Focus the cancel button on open, for dialogs that can lose work. */
	autoFocusCancel?: boolean;
}

export function BaseDialog(props: BaseDialogProps) {
	const confirming = props.confirmLabel !== undefined && props.onConfirm;
	return (
		<Modal
			isOpen={props.open}
			isDismissable={!props.busy}
			isKeyboardDismissDisabled={props.busy}
			onOpenChange={(isOpen) => {
				if (!isOpen) props.onClose();
			}}
		>
			{props.title && <DialogHeading>{props.title}</DialogHeading>}
			<div className="space-y-3 px-5 py-4">
				{props.children}
				{props.error && (
					<p className="leading-6 text-danger mt-2">{props.error}</p>
				)}
			</div>
			<div className="flex justify-end gap-2 border-t border-ui-border px-5 py-3">
				{props.actions ??
					(confirming ? (
						<>
							<Button
								variant="primary"
								autoFocus={props.autoFocusCancel}
								isDisabled={props.busy}
								onPress={props.onClose}
							>
								{props.cancelLabel ?? "Cancel"}
							</Button>
							<Button
								variant={props.confirmVariant ?? "primary"}
								isDisabled={props.busy || props.confirmDisabled}
								onPress={props.onConfirm}
							>
								{props.confirmLabel}
							</Button>
						</>
					) : (
						<Button
							variant="primary"
							isDisabled={props.busy}
							onPress={props.onClose}
						>
							{props.closeLabel ?? "Close"}
						</Button>
					))}
			</div>
		</Modal>
	);
}
