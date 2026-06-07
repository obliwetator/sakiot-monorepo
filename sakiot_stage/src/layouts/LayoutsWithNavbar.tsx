import { useDispatch } from "react-redux";
import { Outlet } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../app/authedFetch";
import { useAppSelector } from "../app/hooks";
import { useGuildSync } from "../app/useGuildSync";
import type { UserGuilds } from "../Constants";
import ResponsiveAppBar from "../navbar";
import { setGuildSelected } from "../reducers/appSlice";

export function LayoutsWithNavbar() {
	const dispatch = useDispatch();
	const guildSelected = useAppSelector((state) => state.app.guildSelected);
	const { data: authData, isError } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});

	const isLoggedIn = !!authData?.user && !isError;
	const userGuilds = authData?.guilds || null;

	useGuildSync(userGuilds);

	const setIsLoggedIn = (value: boolean | ((prev: boolean) => boolean)) => {
		if (
			value === false ||
			(typeof value === "function" && value(isLoggedIn) === false)
		) {
			window.location.reload();
		}
	};

	const setGuildSelectedAction = (guild: UserGuilds | null) => {
		dispatch(setGuildSelected(guild));
	};

	return (
		<div className="h-full flex flex-col">
			<ResponsiveAppBar
				isLoggedIn={isLoggedIn}
				setIsLoggedIn={setIsLoggedIn}
				guildSelected={guildSelected}
				setGuildSelected={setGuildSelectedAction}
				userGuilds={userGuilds}
			/>
			<div className="flex-1 min-h-0 overflow-auto">
				<Outlet />
			</div>
		</div>
	);
}
