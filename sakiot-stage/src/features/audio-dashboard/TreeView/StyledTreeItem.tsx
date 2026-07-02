import Collapse from "@mui/material/Collapse";
import { styled } from "@mui/material/styles";
import type { TransitionProps } from "@mui/material/transitions";
import {
	TreeItem,
	type TreeItemProps,
	treeItemClasses,
} from "@mui/x-tree-view";
import { animated, useSpring } from "@react-spring/web";

function TransitionComponent(props: TransitionProps) {
	const style = useSpring({
		from: { opacity: 0, transform: "translate3d(20px,0,0)" },
		to: {
			opacity: props.in ? 1 : 0,
			transform: `translate3d(${props.in ? 0 : 2}px,0,0)`,
		},
	});

	return (
		<animated.div style={style}>
			<Collapse {...props} />
		</animated.div>
	);
}

export const StyledTreeItem = styled((props: TreeItemProps) => (
	<TreeItem {...props} slots={{ groupTransition: TransitionComponent }} />
))(() => ({
	[`& .${treeItemClasses.iconContainer}`]: {
		"& .close": { opacity: 0.3 },
	},
}));
