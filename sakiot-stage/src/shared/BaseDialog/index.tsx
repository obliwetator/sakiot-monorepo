import type { ReactNode } from "react";
import { Button, DialogHeading, Modal } from "../ui";

export function BaseDialog(props: {
	open: boolean;
	onClose: () => void;
	title?: string;
	error?: string;
	busy?: boolean;
	children: ReactNode;
	actions?: ReactNode;
	closeLabel?: string;
}) {
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
				{props.actions ?? (
					<Button
						variant="primary"
						isDisabled={props.busy}
						onPress={props.onClose}
					>
						{props.closeLabel ?? "Close"}
					</Button>
				)}
			</div>
		</Modal>
	);
}
