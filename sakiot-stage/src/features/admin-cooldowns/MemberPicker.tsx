import { useEffect, useState } from "react";
import {
	ComboBox,
	Input,
	Label,
	ListBox,
	ListBoxItem,
	Popover,
	Text,
} from "react-aria-components";
import {
	type GuildMember,
	useSearchGuildMembersQuery,
} from "../../app/apiSlice";

const SEARCH_DELAY_MS = 250;

/** `value`, once it has stopped changing for `delayMs`. */
function useSettled(value: string, delayMs: number): string {
	const [settled, setSettled] = useState(value);
	useEffect(() => {
		const timer = setTimeout(() => setSettled(value), delayMs);
		return () => clearTimeout(timer);
	}, [value, delayMs]);
	return settled;
}

function describe(member: GuildMember): string {
	return member.name === member.username
		? member.name
		: `${member.name} (${member.username})`;
}

interface MemberPickerProps {
	guildId: string;
	/** A member was chosen: fill in their user id. */
	onPick: (userId: string) => void;
}

/**
 * Searches the guild's members by name or id. Typing an id by hand stays
 * possible next to it, for members the bot has not seen yet.
 */
export function MemberPicker({ guildId, onPick }: MemberPickerProps) {
	const [input, setInput] = useState("");
	const query = useSettled(input.trim(), SEARCH_DELAY_MS);
	const { currentData: page, isFetching } = useSearchGuildMembersQuery(
		{ guild_id: guildId, q: query },
		{ skip: !guildId || query === "" },
	);
	const members = query === "" ? [] : (page?.members ?? []);

	return (
		<ComboBox
			className="flex min-w-0 flex-col gap-1.5"
			inputValue={input}
			onInputChange={setInput}
			items={members}
			allowsEmptyCollection
			menuTrigger="input"
			onSelectionChange={(key) => {
				if (key === null) return;
				onPick(String(key));
				setInput("");
			}}
		>
			<Label className="text-xs font-semibold tracking-wide text-slate-200">
				Find member
			</Label>
			<Input
				placeholder="Name or user ID"
				className="h-9 min-w-0 w-full rounded-md border border-ui-border bg-slate-950/65 px-3 text-sm text-fg outline-hidden placeholder:text-slate-600 data-[focused]:border-primary"
			/>
			{page && !page.complete && (
				<Text slot="description" className="text-xs text-muted">
					The member list is still incomplete; enter a user ID below if someone
					is missing.
				</Text>
			)}
			<Popover className="z-[70] min-w-(--trigger-width) max-h-72 overflow-auto rounded-md border border-ui-border bg-surface p-1 shadow-2xl">
				<ListBox<GuildMember>
					className="outline-hidden"
					renderEmptyState={() => (
						<p className="px-3 py-2 text-sm text-muted">
							{isFetching ? "Searching…" : "No matching members."}
						</p>
					)}
				>
					{(member) => (
						<ListBoxItem
							id={member.user_id}
							textValue={describe(member)}
							className="cursor-default rounded px-3 py-2 text-sm text-fg outline-hidden data-[focused]:bg-ui-border"
						>
							<span className="block truncate">{describe(member)}</span>
							<span className="block text-xs text-muted">
								{member.user_id}
								{member.is_bot ? " · bot" : ""}
							</span>
						</ListBoxItem>
					)}
				</ListBox>
			</Popover>
		</ComboBox>
	);
}
