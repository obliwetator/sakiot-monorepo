//! Mapping from raw songbird event payloads to recorder commands and opus
//! payload bytes.

use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
};
use songbird::packet::{Packet, PacketSize, rtp::RtpExtensionPacket};

use super::RecorderCommand;
use crate::events::voice_receiver::disconnect::{self, recording_channel_has_human_members};

pub(in crate::events::voice_receiver) fn extract_opus_payload(
    data: &songbird::events::context_data::VoiceData,
) -> Option<Vec<u8>> {
    data.packet.as_ref().map(|rtp| {
        let view = rtp.rtp();
        let payload = view.payload();
        let start = rtp.payload_offset.min(payload.len());
        let end = rtp.payload_end_pad.min(payload.len());
        if end <= start {
            return Vec::new();
        }
        let body = &payload[start..end];
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

pub(in crate::events::voice_receiver) fn disconnect_command(
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    data: &songbird::events::context_data::DisconnectData<'_>,
    at_ms: i64,
) -> RecorderCommand {
    use songbird::events::context_data::{DisconnectKind, DisconnectReason};

    let should_count_disconnect = matches!(data.kind, DisconnectKind::Runtime)
        || !matches!(data.reason, Some(DisconnectReason::Requested));
    let channel_has_human_members = recording_channel_has_human_members(ctx, guild_id, channel_id);
    let finalize_empty_channel = disconnect::should_finalize_empty_channel_disconnect(
        data.reason.as_ref(),
        channel_has_human_members,
    );
    let recoverable = disconnect::should_resume_recordings_for_disconnect(
        data.reason.as_ref(),
        disconnect::RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING,
    ) && !finalize_empty_channel;

    RecorderCommand::DriverDisconnected {
        should_count_disconnect,
        recoverable,
        finalize_empty_channel,
        at_ms,
    }
}
