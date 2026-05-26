use songbird::events::context_data::VoiceTick;
use songbird::packet::{Packet, PacketSize, rtp::RtpExtensionPacket};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::error;

use super::InnerReceiver;

pub(super) async fn handle_voice_tick(inner: &Arc<InnerReceiver>, tick: &VoiceTick) {
    // Snapshot the active SSRCs (skip bots).
    let active: Vec<(u32, Arc<tokio::sync::Mutex<super::state::UserRecording>>)> = {
        let map = inner.ssrc_writer_hashmap.read().await;
        let bots = inner.bot_ssrcs.read().await;
        map.iter()
            .filter(|(s, _)| !bots.contains(s))
            .map(|(s, w)| (*s, w.clone()))
            .collect()
    };

    for (ssrc, recording) in active {
        let speaking_data = tick.speaking.get(&ssrc);
        let opus_bytes = speaking_data.and_then(extract_opus_payload);

        let mut rec = recording.lock().await;
        let result = match opus_bytes.as_deref() {
            Some(bytes) if !bytes.is_empty() => {
                let now = chrono::Utc::now().timestamp_millis();
                inner.last_voice_packet_time.store(now, Ordering::Relaxed);
                inner.metrics.track_last_voice_packet(
                    &inner.guild_metrics,
                    &inner.channel_metrics,
                    now,
                );
                inner
                    .metrics
                    .track_audio_packet_received(&inner.guild_metrics, &inner.channel_metrics);
                rec.writer.write_packet(bytes)
            }
            _ => rec.writer.write_silence(1),
        };
        if let Err(e) = result {
            error!("Writer error for ssrc {}: {}", ssrc, e);
        }
    }
}

fn extract_opus_payload(d: &songbird::events::context_data::VoiceData) -> Option<Vec<u8>> {
    d.packet.as_ref().map(|rtp| {
        let view = rtp.rtp();
        let payload = view.payload();
        // NB: in songbird 0.6 VoiceTick, `payload_end_pad` is an
        // absolute end index into `payload`, not a tail-pad count
        // (see songbird/src/driver/tasks/udp_rx/ssrc_state.rs:86).
        let start = rtp.payload_offset.min(payload.len());
        let end = rtp.payload_end_pad.min(payload.len());
        if end <= start {
            return Vec::new();
        }
        let body = &payload[start..end];
        // RTP header extension (Discord uses one-byte form) sits
        // inside the body — skip it before handing bytes to Opus.
        let opus = if view.get_extension() != 0 {
            match RtpExtensionPacket::new(body) {
                Some(ext) => {
                    let off = ext.packet_size();
                    if off >= body.len() {
                        &[][..]
                    } else {
                        &body[off..]
                    }
                }
                None => body,
            }
        } else {
            body
        };
        opus.to_vec()
    })
}
