import Box from "@mui/material/Box";
import Grid from "@mui/material/Grid";
import { useDispatch } from "react-redux";
import { Outlet, useNavigate } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../app/authedFetch";
import { useAppSelector } from "../app/hooks";
import { setGuildSelected } from "../reducers/appSlice";

export const ProtectedLayout = () => {
	const navigate = useNavigate();
	const dispatch = useDispatch();

	const guildSelected = useAppSelector((state) => state.app.guildSelected);
	const { data: authData } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});

	const userGuilds = authData?.guilds || null;

	const handleGuildSelect = (index: number) => {
		if (userGuilds) {
			const value = userGuilds[index];
			dispatch(setGuildSelected(value));
			navigate(value.id);
		}
	};

	if (!userGuilds) {
		return <>No guilds</>;
	}

	const guilds = userGuilds.map((value, index) => {
		return (
			<Grid size={{ xs: 1 }} key={value.id}>
				<button
					type="button"
					onClick={() => handleGuildSelect(index)}
					className="cursor-pointer bg-transparent border-none p-0 text-inherit font-inherit"
				>
					{value.name}
				</button>
			</Grid>
		);
	});

	if (!guildSelected) {
		return (
			<Box sx={{ flexGrow: 1 }}>
				SELECT A SERVER
				<Grid
					container
					justifyContent="center"
					alignItems="center"
					minHeight={300}
				>
					{guilds}
				</Grid>
			</Box>
		);
	}

	return <Outlet />;
};
