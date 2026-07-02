import Button from "@mui/material/Button";
import Dialog from "@mui/material/Dialog";
import DialogActions from "@mui/material/DialogActions";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import Typography from "@mui/material/Typography";
import type { ReactNode } from "react";

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
		<Dialog open={props.open} onClose={props.onClose}>
			{props.title && <DialogTitle>{props.title}</DialogTitle>}
			<DialogContent>
				{props.children}
				{props.error && (
					<Typography color="error" sx={{ mt: 1 }}>
						{props.error}
					</Typography>
				)}
			</DialogContent>
			<DialogActions>
				{props.actions ?? (
					<Button onClick={props.onClose} disabled={props.busy}>
						{props.closeLabel ?? "Close"}
					</Button>
				)}
			</DialogActions>
		</Dialog>
	);
}
