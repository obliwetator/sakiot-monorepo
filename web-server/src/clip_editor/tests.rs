use super::{
    DELAY_MAX_SECONDS, MAX_SEGMENTS, MAX_SHARED_DSP_SEGMENT_SECONDS, MAX_TOTAL_SECONDS,
    SegmentRender, build_filter_graph, build_shared_mix_graph, compose_progress_percent,
    expected_duration_ms, is_valid_clip_id, probe_duration, render_compose_shared,
    shared_dsp_capable, shared_effects_from_dto, validate_edit,
};
use crate::audio::types::WaveformProgressContainer;
use crate::clip_editor::{
    AdvancedSegmentEffectsDto, ComposeClipBody, ComposeLimitsDto, ComposeSegment, SegmentEffectsDto,
};
use actix_web::web;
use std::{collections::HashMap, path::PathBuf};

fn segment_render() -> SegmentRender {
    SegmentRender {
        path: PathBuf::from("src.ogg"),
        source_in: 1.0,
        source_out: 5.0,
        effects: sakiot_dsp::SegmentEffects {
            rate: 2.0,
            pitch_cents: 1200.0,
            volume_db: 6.0,
            bass_db: -3.0,
            mid_db: 2.0,
            treble_db: 3.0,
            ..sakiot_dsp::SegmentEffects::default()
        },
        timeline_start: 2.5,
    }
}

fn body(segments: Vec<ComposeSegment>) -> ComposeClipBody {
    ComposeClipBody {
        name: None,
        master_volume_db: 0.0,
        segments,
        overwrite_clip_id: None,
        limits: None,
    }
}

fn segment() -> ComposeSegment {
    ComposeSegment {
        source: "clip".into(),
        source_id: "abc".into(),
        source_in: 0.0,
        source_out: 10.0,
        timeline_start: 0.0,
        track: 0,
        effects: SegmentEffectsDto {
            volume_db: 0.0,
            pitch_cents: 0.0,
            rate: 1.0,
            bass_db: 0.0,
            mid_db: 0.0,
            treble_db: 0.0,
            reverse: false,
            advanced: AdvancedSegmentEffectsDto::default(),
        },
        merge_group: None,
    }
}

#[test]
fn builds_filter_graph_with_pitch_and_rate() {
    let graph = build_filter_graph(&[segment_render()], -6.0);
    assert!(graph.contains("atrim=start=1.000000:end=5.000000"));
    // Tempo and pitch are independent: 2x is half the duration while
    // +1200 cents raises the result one octave without another resize.
    assert!(graph.contains("rubberband=tempo=2.000000:pitch=2.000000"));
    // 10^(6/20) = 1.995262
    assert!(graph.contains("volume=1.995262"));
    assert!(graph.contains("bass=g=-3.000:f=250:t=s:w=1"));
    assert!(graph.contains("equalizer=g=2.000:f=1000:t=q:w=1"));
    assert!(graph.contains("treble=g=3.000:f=3000:t=s:w=1"));
    assert!(graph.contains("adelay=2500:all=1"));
    assert!(graph.contains("amix=inputs=1:duration=longest:normalize=0"));
    // master volume 10^(-6/20) = 0.501187
    assert!(graph.ends_with("volume=0.501187[out]"));
}

#[test]
fn builds_filter_graph_without_effects() {
    let mut render = segment_render();
    render.effects.rate = 1.0;
    render.effects.pitch_cents = 0.0;
    render.effects.volume_db = 0.0;
    let graph = build_filter_graph(&[render], 0.0);
    assert!(!graph.contains("rubberband"));
    assert!(graph.contains("volume=1.000000"));
}

#[test]
fn builds_filter_graph_with_reverse_after_the_trim() {
    let mut render = segment_render();
    render.effects.reverse = true;
    let graph = build_filter_graph(&[render], 0.0);
    // The window is trimmed first and then flipped, so the audible
    // content is [source_in, source_out] played backwards.
    assert!(graph.contains("atrim=start=1.000000:end=5.000000,areverse"));
    assert!(graph.contains("adelay=2500:all=1"));
}

#[test]
fn forward_segments_omit_the_reverse_filter() {
    let graph = build_filter_graph(&[segment_render()], 0.0);
    assert!(!graph.contains("areverse"));
}

#[test]
fn mixes_two_segments_into_one_output() {
    let graph = build_filter_graph(&[segment_render(), segment_render()], 0.0);
    assert!(graph.contains("[s0][s1]amix=inputs=2:duration=longest:normalize=0"));
}

#[test]
fn shared_mix_graph_only_places_and_sums_preprocessed_segments() {
    let graph = build_shared_mix_graph(&[segment_render(), segment_render()], -6.0);
    assert!(graph.contains("[0:a]adelay=2500:all=1[s0]"));
    assert!(graph.contains("[1:a]adelay=2500:all=1[s1]"));
    assert!(graph.contains("[s0][s1]amix=inputs=2:duration=longest:normalize=0"));
    assert!(graph.contains("channel_layouts=stereo"));
    assert!(graph.ends_with("volume=0.501187[out]"));
    assert!(!graph.contains("rubberband"));
    assert!(!graph.contains("equalizer"));
}

#[test]
fn maps_product_effects_into_the_shared_contract() {
    let mut dto = segment().effects;
    dto.volume_db = 6.0;
    dto.pitch_cents = 700.0;
    dto.rate = 1.35;
    dto.bass_db = -3.0;
    dto.mid_db = 2.0;
    dto.treble_db = 3.0;
    dto.reverse = true;
    dto.advanced.tail_seconds = 2.0;
    dto.advanced.distortion_amount = 0.8;
    dto.advanced.distortion_wet = 0.6;
    dto.advanced.delay_seconds = 0.4;
    dto.advanced.delay_feedback = 0.35;
    dto.advanced.delay_wet = 0.25;
    dto.advanced.compressor_enabled = true;
    dto.advanced.chorus_enabled = true;
    dto.advanced.reverb_enabled = true;
    dto.advanced.reverb_seed = 42;
    let effects = shared_effects_from_dto(&dto);
    assert_eq!(effects.volume_db, 6.0);
    assert_eq!(effects.pitch_cents, 700.0);
    assert_eq!(effects.rate, 1.35);
    assert_eq!(effects.bass_db, -3.0);
    assert_eq!(effects.mid_db, 2.0);
    assert_eq!(effects.treble_db, 3.0);
    assert!(effects.reverse);
    assert_eq!(effects.tail_seconds, 2.0);
    assert_eq!(effects.distortion_amount, 0.8);
    assert_eq!(effects.distortion_wet, 0.6);
    assert_eq!(effects.delay_seconds, 0.4);
    assert_eq!(effects.delay_feedback, 0.35);
    assert_eq!(effects.delay_wet, 0.25);
    assert!(effects.compressor_enabled);
    assert!(effects.chorus_enabled);
    assert!(effects.reverb_enabled);
    assert_eq!(effects.reverb_seed, 42);
}

#[actix_rt::test]
#[ignore = "manual real-media integration check"]
async fn renders_real_clip_through_shared_server_pipeline() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sakiot-DSP/examples/8161a145-9b3d-4bdb-bca1-4d12fd6781a2/source.ogg");
    assert!(source.is_file(), "real-media fixture is missing");
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("shared-render.ogg");
    let mut render = segment_render();
    render.path = source;
    render.source_in = 0.25;
    render.source_out = 5.25;
    render.timeline_start = 0.125;
    render.effects.rate = 1.35;
    render.effects.pitch_cents = 700.0;
    render.effects.volume_db = -3.0;
    render.effects.reverse = true;
    render.effects.tail_seconds = 0.2;
    render.effects.distortion_amount = 0.6;
    render.effects.distortion_wet = 0.25;
    render.effects.delay_seconds = 0.05;
    render.effects.delay_feedback = 0.2;
    render.effects.delay_wet = 0.2;
    render.effects.compressor_enabled = true;
    render.effects.chorus_enabled = true;
    render.effects.reverb_enabled = true;
    render.effects.reverb_decay_seconds = 0.2;
    render.effects.reverb_wet = 0.2;
    render.effects.reverb_seed = 42;
    let expected_ms = expected_duration_ms(std::slice::from_ref(&render));
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        HashMap::new(),
    )));

    render_compose_shared(
        &[render],
        -2.0,
        &output,
        expected_ms,
        &progress,
        "real-media-test",
    )
    .await
    .unwrap();

    let duration = probe_duration(&output).await.unwrap();
    let expected_seconds =
        std::time::Duration::from_millis(u64::try_from(expected_ms).unwrap()).as_secs_f64();
    assert!((duration - expected_seconds).abs() < 0.1);
    assert!(std::fs::metadata(output).unwrap().len() > 1_000);
}

#[test]
fn maps_ffmpeg_output_time_into_composition_progress() {
    assert_eq!(
        compose_progress_percent("out_time_us=5000000", 10_000),
        Some(49)
    );
    assert_eq!(
        compose_progress_percent("out_time_us=10000000", 10_000),
        Some(99)
    );
    assert_eq!(compose_progress_percent("progress=end", 10_000), None);
}

#[test]
fn computes_expected_duration_from_the_longest_segment() {
    // Content 1.0..5.0 = 4s at 2x tempo -> 2s. The +1200-cent pitch
    // shift preserves that duration, so the timeline ends at 4.5s.
    assert_eq!(expected_duration_ms(&[segment_render()]), 4_500);
}

#[test]
fn effect_tail_extends_the_expected_timeline_after_rate_processing() {
    let mut render = segment_render();
    render.effects.tail_seconds = 2.0;
    // Four content seconds at 2x consume 2s; the fixed 2s tail is not
    // rate-scaled, and the segment starts at 2.5s.
    assert_eq!(expected_duration_ms(&[render]), 6_500);
}

#[test]
fn routes_extreme_pitch_or_rate_stretches_off_the_shared_dsp() {
    // Default segment: 2x pitch over 2x rate = 1x stretch, in bounds.
    assert!(shared_dsp_capable(&[segment_render()]));
    let mut render = segment_render();
    render.effects.pitch_cents = 4_800.0; // 16x pitch, 2x rate = 8x stretch.
    assert!(shared_dsp_capable(&[render.clone()]));
    render.effects.rate = 0.25; // 16/0.25 = 64x stretch -> legacy path.
    assert!(!shared_dsp_capable(&[render]));
    let mut render = segment_render();
    render.effects.pitch_cents = 0.0;
    render.effects.rate = 0.1; // 1/0.1 = 10x stretch, still in bounds.
    assert!(shared_dsp_capable(&[render.clone()]));
    render.effects.pitch_cents = 2_400.0; // 4/0.1 = 40x -> legacy path.
    assert!(!shared_dsp_capable(&[render]));
    let mut render = segment_render();
    render.source_out = MAX_SHARED_DSP_SEGMENT_SECONDS + 10.0;
    assert!(!shared_dsp_capable(&[render]));
}

#[test]
fn expected_duration_ignores_pitch_and_follows_speed() {
    let mut render = segment_render();
    render.effects.pitch_cents = -1200.0;
    render.source_in = 0.0;
    render.source_out = 10.0;
    render.timeline_start = 0.0;
    // Pitch is duration-preserving; only the 2x tempo resizes the clip.
    assert_eq!(expected_duration_ms(&[render]), 5_000);
}

#[test]
fn validates_clip_ids() {
    assert!(is_valid_clip_id("abc-123"));
    assert!(!is_valid_clip_id(""));
    assert!(!is_valid_clip_id("../secret"));
    assert!(!is_valid_clip_id("a/b"));
    assert!(!is_valid_clip_id("a\\b"));
    assert!(!is_valid_clip_id("a\nb"));
}

#[test]
fn rejects_empty_and_oversized_compositions() {
    assert!(validate_edit(&body(vec![])).is_err());
    assert!(validate_edit(&body(vec![segment(); MAX_SEGMENTS + 1])).is_err());
}

#[test]
fn accepts_plain_composed_clip_overwrite_ids() {
    let mut edit = body(vec![segment()]);
    edit.overwrite_clip_id = Some("abc-123".into());
    assert!(validate_edit(&edit).is_ok());
}

#[test]
fn rejects_unsafe_overwrite_clip_ids() {
    for id in ["", "../secret", "a/b", "a\\b", "a\nb"] {
        let mut edit = body(vec![segment()]);
        edit.overwrite_clip_id = Some(id.into());
        assert!(validate_edit(&edit).is_err(), "id {id:?} was accepted");
    }
}

#[test]
fn rejects_non_clip_sources() {
    let mut seg = segment();
    seg.source = "session".into();
    assert!(validate_edit(&body(vec![seg])).is_err());
}

#[test]
fn rejects_invalid_merge_group_ids() {
    let mut seg = segment();
    seg.merge_group = Some("../escape".into());
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.merge_group = Some("".into());
    assert!(validate_edit(&body(vec![seg])).is_err());
}

#[test]
fn merge_groups_round_trip_through_the_stored_json() {
    let mut first = segment();
    first.merge_group = Some("group-1".into());
    let mut second = segment();
    second.source_id = "def".into();
    second.source_out = 20.0;
    second.timeline_start = 10.0;
    second.merge_group = Some("group-1".into());
    let value = serde_json::to_value(body(vec![first, second])).unwrap();
    assert_eq!(value["segments"][0]["merge_group"], "group-1");
    assert_eq!(value["segments"][1]["merge_group"], "group-1");
    let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
    assert_eq!(restored.segments[0].merge_group.as_deref(), Some("group-1"));
    assert_eq!(restored.segments[1].merge_group.as_deref(), Some("group-1"));
}

#[test]
fn merge_group_defaults_to_none_when_missing() {
    let mut value = serde_json::to_value(body(vec![segment()])).unwrap();
    value["segments"][0]
        .as_object_mut()
        .unwrap()
        .remove("merge_group");
    let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
    assert_eq!(restored.segments[0].merge_group, None);
}

#[test]
fn rejects_out_of_bounds_effects() {
    let mut seg = segment();
    seg.effects.volume_db = 25.0;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.volume_db = 0.0;
    seg.effects.pitch_cents = 2401.0;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.pitch_cents = 0.0;
    seg.effects.rate = 0.24;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.rate = 1.0;
    seg.effects.bass_db = -25.0;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.bass_db = 0.0;
    seg.effects.advanced.distortion_wet = 1.1;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.advanced.distortion_wet = 0.0;
    seg.effects.advanced.tail_seconds = sakiot_dsp::MAX_EFFECT_TAIL_SECONDS + 0.1;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.advanced.tail_seconds = 0.0;
    seg.effects.advanced.delay_seconds = DELAY_MAX_SECONDS + 0.1;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.advanced.delay_seconds = 0.25;
    seg.effects.advanced.compressor_ratio = 21.0;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.advanced.compressor_ratio = 12.0;
    seg.effects.advanced.chorus_depth = -0.1;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.effects.advanced.chorus_depth = 0.7;
    seg.effects.advanced.reverb_decay_seconds = 0.0;
    assert!(validate_edit(&body(vec![seg])).is_err());
}

#[test]
fn accepts_the_doubled_default_limits() {
    let mut seg = segment();
    seg.effects.volume_db = 24.0;
    seg.effects.pitch_cents = -2400.0;
    seg.effects.rate = 4.0;
    seg.effects.mid_db = -24.0;
    seg.effects.treble_db = 24.0;
    assert!(validate_edit(&body(vec![seg])).is_ok());
}

#[test]
fn custom_limits_override_the_defaults() {
    let mut seg = segment();
    seg.effects.volume_db = 100.0;
    let mut edit = body(vec![seg]);
    edit.limits = Some(ComposeLimitsDto {
        volume_db_max: 120.0,
        ..ComposeLimitsDto::default()
    });
    assert!(validate_edit(&edit).is_ok());
}

#[test]
fn rejects_limits_outside_the_safety_caps() {
    let limit_cases: Vec<ComposeLimitsDto> = vec![
        ComposeLimitsDto {
            volume_db_max: 241.0,
            ..ComposeLimitsDto::default()
        },
        ComposeLimitsDto {
            pitch_cents_max: 4801.0,
            ..ComposeLimitsDto::default()
        },
        ComposeLimitsDto {
            rate_min: 0.05,
            ..ComposeLimitsDto::default()
        },
        ComposeLimitsDto {
            bass_db_min: -241.0,
            ..ComposeLimitsDto::default()
        },
    ];
    for limits in limit_cases {
        let mut edit = body(vec![segment()]);
        edit.limits = Some(limits);
        assert!(validate_edit(&edit).is_err());
    }
}

#[test]
fn rejects_inverted_or_non_finite_limits() {
    for limits in [
        ComposeLimitsDto {
            volume_db_min: 0.0,
            volume_db_max: -1.0,
            ..ComposeLimitsDto::default()
        },
        ComposeLimitsDto {
            rate_min: f32::NAN,
            ..ComposeLimitsDto::default()
        },
    ] {
        let mut edit = body(vec![segment()]);
        edit.limits = Some(limits);
        assert!(validate_edit(&edit).is_err());
    }
}

#[test]
fn rejects_invalid_source_ranges_and_timeline_starts() {
    let mut seg = segment();
    seg.source_out = 0.0;
    assert!(validate_edit(&body(vec![seg.clone()])).is_err());
    seg.source_out = 10.0;
    seg.timeline_start = -1.0;
    assert!(validate_edit(&body(vec![seg])).is_err());
}

#[test]
fn rejects_compositions_longer_than_the_sanity_limit() {
    let mut seg = segment();
    seg.timeline_start = MAX_TOTAL_SECONDS as f32;
    assert!(validate_edit(&body(vec![seg])).is_err());
}

#[test]
fn rejects_advanced_effects_on_oversized_in_memory_segments() {
    let mut seg = segment();
    seg.source_out = MAX_SHARED_DSP_SEGMENT_SECONDS + 1.0;
    seg.effects.advanced.distortion_wet = 0.5;
    assert!(validate_edit(&body(vec![seg])).is_err());
}

#[test]
fn stored_edit_json_round_trips_the_request_shape() {
    let value = serde_json::to_value(body(vec![segment()])).unwrap();
    assert_eq!(value["master_volume_db"], 0.0);
    assert_eq!(value["segments"][0]["source"], "clip");
    assert_eq!(value["segments"][0]["source_id"], "abc");
    assert_eq!(value["segments"][0]["source_in"], 0.0);
    assert_eq!(value["segments"][0]["source_out"], 10.0);
    assert_eq!(value["segments"][0]["timeline_start"], 0.0);
    assert_eq!(value["segments"][0]["track"], 0);
    assert_eq!(value["segments"][0]["effects"]["volume_db"], 0.0);
    assert_eq!(value["segments"][0]["effects"]["pitch_cents"], 0.0);
    assert_eq!(value["segments"][0]["effects"]["rate"], 1.0);
    assert_eq!(value["segments"][0]["effects"]["bass_db"], 0.0);
    assert_eq!(value["segments"][0]["effects"]["mid_db"], 0.0);
    assert_eq!(value["segments"][0]["effects"]["treble_db"], 0.0);
    let distortion_amount = value["segments"][0]["effects"]["advanced"]["distortion_amount"]
        .as_f64()
        .unwrap();
    assert!((distortion_amount - 0.4).abs() < 1e-6);
    assert_eq!(
        value["segments"][0]["effects"]["advanced"]["tail_seconds"],
        0.0
    );
    assert_eq!(
        value["segments"][0]["effects"]["advanced"]["reverb_seed"],
        0x5341_4b49_u32
    );
    assert_eq!(value["segments"][0]["effects"]["reverse"], false);
}

#[test]
fn effects_default_to_forward_playback_when_reverse_is_missing() {
    let mut seg = segment();
    let mut value = serde_json::to_value(body(vec![seg.clone()])).unwrap();
    value["segments"][0]["effects"]
        .as_object_mut()
        .unwrap()
        .remove("reverse");
    let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
    assert!(!restored.segments[0].effects.reverse);
    seg.effects.reverse = true;
    let value = serde_json::to_value(body(vec![seg])).unwrap();
    let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
    assert!(restored.segments[0].effects.reverse);
}

#[test]
fn effects_default_to_flat_mid_eq_when_mid_is_missing() {
    let mut value = serde_json::to_value(body(vec![segment()])).unwrap();
    value["segments"][0]["effects"]
        .as_object_mut()
        .unwrap()
        .remove("mid_db");
    let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
    assert_eq!(restored.segments[0].effects.mid_db, 0.0);
}

#[test]
fn advanced_effects_default_to_shared_bypass_when_missing() {
    let mut value = serde_json::to_value(body(vec![segment()])).unwrap();
    value["segments"][0]["effects"]
        .as_object_mut()
        .unwrap()
        .remove("advanced");
    let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
    let advanced = &restored.segments[0].effects.advanced;
    assert_eq!(advanced.distortion_amount, 0.4);
    assert_eq!(advanced.tail_seconds, 0.0);
    assert_eq!(advanced.distortion_wet, 0.0);
    assert_eq!(advanced.delay_wet, 0.0);
    assert!(!advanced.compressor_enabled);
    assert!(!advanced.chorus_enabled);
    assert!(!advanced.reverb_enabled);
    assert_eq!(advanced.reverb_seed, 0x5341_4b49);
}
