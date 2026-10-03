import { Button, Notice } from "./ui";

interface SavedValueChangedProps {
	/** The newer saved value, as the user would read it. */
	saved: string;
	onTakeSaved: () => void;
	onSaveMine: () => void;
	isSaving: boolean;
}

/**
 * Shown under a field whose saved value changed (another admin saved) while
 * the user was editing it. Neither side is lost until the user picks one.
 */
export function SavedValueChanged({
	saved,
	onTakeSaved,
	onSaveMine,
	isSaving,
}: SavedValueChangedProps) {
	return (
		<Notice tone="warning" announce="status">
			<span>Someone else saved {saved} while you were editing.</span>
			<div className="mt-2 flex flex-wrap gap-2">
				<Button size="sm" variant="outline" onPress={onTakeSaved}>
					Use saved value
				</Button>
				<Button
					size="sm"
					variant="primary"
					isDisabled={isSaving}
					onPress={onSaveMine}
				>
					Save my version
				</Button>
			</div>
		</Notice>
	);
}
