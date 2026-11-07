import { useGetBotLogQuery } from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import { ScrollRegion } from "../../shared/ui";
import { when } from "../kept-recordings/keeping";
import { culprit, happened } from "./describeBotLog";

/** What others did to the bot in voice, newest first. */
export function BotLogPanel({ guildId }: { guildId: string }) {
	const query = useGetBotLogQuery(guildId, { skip: !guildId });

	return (
		<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-6">
			<h6 className="font-medium tracking-[0.001em] text-xl mb-2">Bot log</h6>
			{query.isError && (
				<LoadFailure
					error={query.error}
					what="Could not load the bot log."
					onRetry={() => void query.refetch()}
				/>
			)}
			{query.data && (
				<div className="flex flex-col gap-4">
					<p className="leading-6 text-muted">
						When someone else deafens, undeafens, moves or disconnects the bot.
						A deafened bot hears nothing, so it records silence. Discord only
						lets a deafen be removed from someone in voice, so a deafen the bot
						left with is removed when it next joins.
					</p>
					{query.data.length === 0 ? (
						<p className="leading-6 text-muted">Nothing yet.</p>
					) : (
						<ScrollRegion aria-label="Bot log, scrollable">
							<ul aria-label="Bot log" className="flex flex-col gap-3">
								{query.data.map((entry) => {
									const who = culprit(entry);
									return (
										<li key={entry.id} className="flex flex-col">
											<span>{happened(entry)}</span>
											<span className="text-sm text-muted">
												{when(entry.occurred_at_ms)}
												{who && ` · ${who}`}
											</span>
										</li>
									);
								})}
							</ul>
						</ScrollRegion>
					)}
				</div>
			)}
		</div>
	);
}
