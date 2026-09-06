import type * as React from "react";
import type { UserGuilds } from "../Constants";
import Login from "../login/login";
import { GuildSelect } from "../shared/GuildSelect";
import { Avatar, Button } from "../shared/ui";
import { type PageName, pageIcons } from "./constants";

export function MobileDrawer(props: {
	isLoggedIn: boolean;
	setIsLoggedIn: React.Dispatch<React.SetStateAction<boolean>>;
	guildSelected: UserGuilds | null;
	setGuildSelected: (guild: UserGuilds | null) => void;
	userGuilds: UserGuilds[] | null;
	visiblePages: PageName[];
	onNavigate: (name: PageName) => void;
}) {
	return (
		<div role="presentation" className="w-70">
			<div className="p-4 flex items-center gap-4">
				<Avatar alt="user" src="/pepega.png" />
				<h6 className="text-base truncate">
					{props.isLoggedIn ? "Account" : "Guest"}
				</h6>
			</div>
			<hr className="w-full border-t border-ui-border" />
			{props.userGuilds && props.userGuilds.length > 0 ? (
				<div className="p-4">
					<GuildSelect
						guildSelected={props.guildSelected}
						setGuildSelected={props.setGuildSelected}
						userGuilds={props.userGuilds}
					/>
				</div>
			) : null}
			<hr className="w-full border-t border-ui-border" />
			<div className="flex flex-col gap-1">
				<div className={"px-4 py-2 text-xs font-semibold uppercase text-muted"}>
					Navigate
				</div>
				{props.visiblePages.map((page) => (
					<div key={page} className="relative flex items-center">
						<Button
							className="w-full justify-start text-left"
							variant="ghost"
							onPress={() => props.onNavigate(page)}
						>
							<span className="mr-2 inline-flex min-w-8 items-center text-muted">
								{pageIcons[page]}
							</span>
							<div>{page}</div>
						</Button>
					</div>
				))}
			</div>
			<hr className="w-full border-t border-ui-border" />
			<div className="flex flex-col gap-1">
				<div className={"px-4 py-2 text-xs font-semibold uppercase text-muted"}>
					Account
				</div>
				<div className="relative flex items-center">
					<div className="px-4 py-2">
						<Login
							isLoggedIn={props.isLoggedIn}
							setIsLoggedIn={props.setIsLoggedIn}
						/>
					</div>
				</div>
			</div>
		</div>
	);
}
