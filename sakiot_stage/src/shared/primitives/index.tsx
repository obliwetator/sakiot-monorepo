import Box from "@mui/material/Box";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import Paper from "@mui/material/Paper";
import Typography from "@mui/material/Typography";
import type { ReactNode } from "react";

export function Tile({
	label,
	value,
	warn,
}: {
	label: string;
	value: ReactNode;
	warn?: boolean;
}) {
	return (
		<Box
			sx={{
				flex: "1 1 130px",
				maxWidth: 200,
				p: 1.5,
				borderRadius: 1,
				bgcolor: "background.paper",
				border: warn ? "1px solid #f44336" : "1px solid",
				borderColor: warn ? "#f44336" : "divider",
			}}
		>
			<Typography
				fontSize={11}
				color="text.secondary"
				gutterBottom
				sx={{ textTransform: "uppercase", letterSpacing: 0.5 }}
			>
				{label}
			</Typography>
			<Typography variant="h6" fontSize={18} fontWeight={600}>
				{value}
			</Typography>
		</Box>
	);
}

export function StatCard({
	label,
	value,
}: {
	label: string;
	value: ReactNode;
}) {
	return (
		<Box sx={{ flex: "1 1 160px" }}>
			<Card>
				<CardContent>
					<Typography color="text.secondary" fontSize={13} gutterBottom>
						{label}
					</Typography>
					<Typography variant="h5" fontWeight={600}>
						{value}
					</Typography>
				</CardContent>
			</Card>
		</Box>
	);
}

export function GroupPanel({
	title,
	children,
}: {
	title: string;
	children: ReactNode;
}) {
	return (
		<Paper variant="outlined" sx={{ p: 2 }}>
			<Typography
				fontSize={11}
				fontWeight={700}
				color="text.secondary"
				sx={{ textTransform: "uppercase", letterSpacing: 1, mb: 1.5 }}
			>
				{title}
			</Typography>
			<Box sx={{ display: "flex", flexWrap: "wrap", gap: 1.5 }}>{children}</Box>
		</Paper>
	);
}
