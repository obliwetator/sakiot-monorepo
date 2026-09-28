import { problemFromError } from "../app/apiError";
import { Button, cn, Notice } from "./ui";

/**
 * A failed load: what could not be loaded, why (as far as is safely known),
 * and a way to ask again. Loads are reads, so retrying is always safe.
 */
export function LoadFailure(props: {
	error: unknown;
	/** Names what failed, e.g. "Could not load stamps." */
	what: string;
	onRetry: () => void;
	className?: string;
}) {
	const problem = problemFromError(props.error);
	return (
		<div className={cn("flex flex-wrap items-center gap-2", props.className)}>
			<Notice tone="error" announce="alert">
				{`${props.what} ${problem.message}`}
			</Notice>
			<Button variant="outline" onPress={props.onRetry}>
				Retry
			</Button>
		</div>
	);
}
