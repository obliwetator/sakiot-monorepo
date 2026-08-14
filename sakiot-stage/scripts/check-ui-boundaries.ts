import { readdir, readFile } from "node:fs/promises";
import { relative, resolve } from "node:path";

const sourceRoot = resolve(import.meta.dir, "../src");

const muiAllowlist = new Set([
	"src/App.tsx",
	"src/app/BundleUpdatePrompt.tsx",
	"src/app/theme.ts",
	"src/features/admin-voice-settings/index.tsx",
	"src/features/audio-dashboard/AudioEventTimeline.tsx",
	"src/features/audio-dashboard/ChannelMixPlayer.tsx",
	"src/features/audio-dashboard/ChannelMixWaveforms.tsx",
	"src/features/audio-dashboard/ClipRangeEditorView.tsx",
	"src/features/audio-dashboard/ClipRangePrecisionOverlay.tsx",
	"src/features/audio-dashboard/LogicalSessionPanels.tsx",
	"src/features/audio-dashboard/LogicalSessionPlayer.tsx",
	"src/features/audio-dashboard/PlaybackControls.tsx",
	"src/features/audio-dashboard/RangeSlider/ClipDialog.tsx",
	"src/features/audio-dashboard/RangeSlider/DoubleSlider.tsx",
	"src/features/audio-dashboard/RangeSlider/DownloadButton.tsx",
	"src/features/audio-dashboard/RangeSlider/JamIt.tsx",
	"src/features/audio-dashboard/RangeSlider/PlaybackSpeedSlider.tsx",
	"src/features/audio-dashboard/RangeSlider/RangeDetails.tsx",
	"src/features/audio-dashboard/RangeSlider/SilenceButton.tsx",
	"src/features/audio-dashboard/RangeSlider/TimeEditor.tsx",
	"src/features/audio-dashboard/RangeSlider/VolumeSlider.tsx",
	"src/features/audio-dashboard/RangeSlider/index.tsx",
	"src/features/audio-dashboard/SessionPlaybackTimeline.tsx",
	"src/features/audio-dashboard/SessionWaveform.tsx",
	"src/features/audio-dashboard/SilenceFreePlayer.tsx",
	"src/features/audio-dashboard/TreeView/ItemsEl.tsx",
	"src/features/audio-dashboard/TreeView/StyledTreeItem.tsx",
	"src/features/audio-dashboard/TreeView/index.tsx",
	"src/features/audio-dashboard/Waveform.tsx",
	"src/features/audio-dashboard/YearSelection.tsx",
	"src/features/audio-dashboard/timelineLayout.tsx",
	"src/features/clip-editor/ClipBin.tsx",
	"src/features/clip-editor/ClipEditor.tsx",
	"src/features/clip-editor/EditorOptionsDialog.tsx",
	"src/features/clip-editor/EffectLimitsDialog.tsx",
	"src/features/clip-editor/EffectSettingsJsonDialog.tsx",
	"src/features/clip-editor/Inspector.tsx",
	"src/features/clip-editor/Timeline.tsx",
	"src/features/clip-editor/TimelineOverlays.tsx",
	"src/features/clip-editor/TimelineRuler.tsx",
	"src/features/clip-editor/TimelineTracks.tsx",
	"src/features/clip-editor/unsavedChangesGuard.tsx",
	"src/features/clips/ClipPlayer.tsx",
	"src/features/clips/ClipWaveform.tsx",
	"src/features/clips/index.tsx",
	"src/features/members/ViewAsRoleBanner.tsx",
	"src/features/members/index.tsx",
	"src/features/stamps/index.tsx",
	"src/layouts/ProtectedLayout.tsx",
	"src/login/login.tsx",
	"src/navbar/MobileDrawer.tsx",
	"src/navbar/UserMenu.tsx",
	"src/navbar/constants.tsx",
	"src/navbar/index.tsx",
	"src/routes/AppRoutes.tsx",
	"src/shared/BaseDialog/index.tsx",
	"src/shared/BasicSelect/index.tsx",
	"src/shared/primitives/index.tsx",
]);

async function sourceFiles(directory: string): Promise<string[]> {
	const entries = await readdir(directory, { withFileTypes: true });
	const nested = await Promise.all(
		entries.map(async (entry) => {
			const path = resolve(directory, entry.name);
			if (entry.isDirectory()) return sourceFiles(path);
			return /\.(?:js|jsx|ts|tsx)$/.test(entry.name) ? [path] : [];
		}),
	);
	return nested.flat();
}

const violations: string[] = [];
for (const path of await sourceFiles(sourceRoot)) {
	const repoPath = relative(resolve(sourceRoot, ".."), path).replaceAll(
		"\\",
		"/",
	);
	const source = await readFile(path, "utf8");
	if (/["']@mui\//.test(source) && !muiAllowlist.has(repoPath)) {
		violations.push(`${repoPath}: new MUI import`);
	}
	if (
		/["']react-aria-components(?:\/|["'])/.test(source) &&
		!repoPath.startsWith("src/shared/ui/")
	) {
		violations.push(`${repoPath}: React Aria must be wrapped by shared/ui`);
	}
}

if (violations.length > 0) {
	console.error(`UI boundary check failed:\n${violations.join("\n")}`);
	process.exit(1);
}

console.log("UI boundary check passed.");
