import ContentCutIcon from "@mui/icons-material/ContentCut";
import DeleteIcon from "@mui/icons-material/Delete";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Divider from "@mui/material/Divider";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import { formatDuration } from "../../utils/formatTime";
import type { SegmentEffects, TimelineSegment } from "./model";
import type { UseClipEditorReturn } from "./useClipEditor";

const SLIDER_PROPS = {
	size: "small" as const,
	sx: { "& .MuiSlider-thumb, & .MuiSlider-track": { transition: "none" } },
};

export function Inspector(props: {
	editor: UseClipEditorReturn;
	clipName: (sourceId: string) => string;
}) {
	const { editor } = props;
	const segment = editor.selectedSegment;

	return (
		<Box
			sx={{
				width: 260,
				flex: "0 0 auto",
				minHeight: 0,
				minWidth: 0,
				overflowY: "auto",
				overflowX: "hidden",
				borderLeft: 1,
				borderColor: "divider",
				p: 2,
			}}
		>
			{segment ? (
				<SegmentInspectorContent
					editor={editor}
					segment={segment}
					clipName={props.clipName}
				/>
			) : (
				<>
					<Typography variant="overline" color="text.secondary">
						Inspector
					</Typography>
					<Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
						No segment selected.
					</Typography>
					<Typography
						variant="caption"
						color="text.secondary"
						display="block"
						sx={{ mt: 0.5 }}
					>
						Click a clip on the timeline to trim it and adjust its effects.
					</Typography>
				</>
			)}
		</Box>
	);
}

function SegmentInspectorContent(props: {
	editor: UseClipEditorReturn;
	segment: TimelineSegment;
	clipName: (sourceId: string) => string;
}) {
	const { editor } = props;
	const segment = props.segment;

	const patchEffects = (patch: Partial<SegmentEffects>) => {
		editor.preview((edit) => ({
			...edit,
			segments: edit.segments.map((s) =>
				s.id === segment.id ? { ...s, effects: { ...s.effects, ...patch } } : s,
			),
		}));
	};

	const finishSlider = () => editor.flush();

	const trimEdge = (edge: "in" | "out") => {
		const at = editor.positionSec - segment.timelineStart;
		editor.apply((edit) => ({
			...edit,
			segments: edit.segments.map((s) => {
				if (s.id !== segment.id) return s;
				const sourceIn = edge === "in" ? Math.max(0, at) : s.sourceIn;
				const sourceOut =
					edge === "out" ? Math.max(sourceIn + 0.05, at) : s.sourceOut;
				return { ...s, sourceIn, sourceOut };
			}),
		}));
	};

	const duration = segment.sourceOut - segment.sourceIn;

	return (
		<>
			<Typography variant="overline" color="text.secondary">
				Selected segment
			</Typography>
			<Typography variant="h6" noWrap title={props.clipName(segment.sourceId)}>
				{props.clipName(segment.sourceId)}
			</Typography>
			<Typography variant="caption" color="text.secondary">
				{formatDuration(duration)} · at {formatDuration(segment.timelineStart)}
			</Typography>

			<Stack direction="row" spacing={1} sx={{ mt: 1.5 }}>
				<Button
					size="small"
					variant="outlined"
					onClick={() => trimEdge("in")}
					title="Set source in-point to the playhead (I)"
				>
					In (I)
				</Button>
				<Button
					size="small"
					variant="outlined"
					onClick={() => trimEdge("out")}
					title="Set source out-point to the playhead (O)"
				>
					Out (O)
				</Button>
			</Stack>

			<Divider sx={{ my: 2 }} />

			<Typography variant="overline" color="text.secondary">
				Effects
			</Typography>

			<EffectSlider
				label="Volume"
				value={segment.effects.volumeDb}
				min={-40}
				max={12}
				step={0.5}
				format={(value) => `${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects({ volumeDb: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				label="Pitch"
				value={segment.effects.pitchCents}
				min={-1200}
				max={1200}
				step={10}
				format={(value) =>
					value === 0 ? "0" : `${value > 0 ? "+" : ""}${value} ct`
				}
				onChange={(value) => patchEffects({ pitchCents: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				label="Speed"
				value={segment.effects.rate}
				min={0.5}
				max={2}
				step={0.05}
				format={(value) => `${value.toFixed(2)}×`}
				onChange={(value) => patchEffects({ rate: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				label="Bass"
				value={segment.effects.bassDb}
				min={-12}
				max={12}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects({ bassDb: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				label="Treble"
				value={segment.effects.trebleDb}
				min={-12}
				max={12}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects({ trebleDb: value })}
				onCommitted={finishSlider}
			/>

			<Divider sx={{ my: 2 }} />

			<Stack direction="row" spacing={1}>
				<Button
					size="small"
					variant="outlined"
					startIcon={<ContentCutIcon />}
					onClick={editor.splitSelectedAtPlayhead}
				>
					Split (S)
				</Button>
				<Button
					size="small"
					variant="outlined"
					color="error"
					startIcon={<DeleteIcon />}
					onClick={editor.removeSelected}
				>
					Delete
				</Button>
			</Stack>
		</>
	);
}

function EffectSlider(props: {
	label: string;
	value: number;
	min: number;
	max: number;
	step: number;
	format: (value: number) => string;
	onChange: (value: number) => void;
	onCommitted: () => void;
}) {
	return (
		<Box sx={{ mb: 1 }}>
			<Typography variant="caption" color="text.secondary">
				{props.label} · {props.format(props.value)}
			</Typography>
			<Slider
				{...SLIDER_PROPS}
				min={props.min}
				max={props.max}
				step={props.step}
				value={props.value}
				onChange={(_event, value) => props.onChange(Number(value))}
				onChangeCommitted={props.onCommitted}
				aria-label={props.label}
			/>
		</Box>
	);
}
