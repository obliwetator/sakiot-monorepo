import { createTheme } from "@mui/material/styles";

export const darkTheme = createTheme({
	palette: {
		mode: "dark",
		background: {
			default: "#18181b",
			paper: "#202023",
		},
		divider: "#3f3f46",
	},
	typography: { fontFamily: '"Inter Variable", system-ui, sans-serif' },
});
