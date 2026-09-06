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
			<div key={value.id}>
				<button
					type="button"
					onClick={() => handleGuildSelect(index)}
					className="cursor-pointer bg-transparent border-none p-0 text-inherit font-inherit"
				>
					{value.name}
				</button>
			</div>
		);
	});

	if (!guildSelected) {
		return (
			<div className="grow">
				SELECT A SERVER
				<div className="justify-center items-center min-h-75 grid [grid-template-columns:repeat(auto-fit,minmax(180px,1fr))]">
					{guilds}
				</div>
			</div>
		);
	}

	return <Outlet />;
};
