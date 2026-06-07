pub mod stamp;
pub mod voice_controls;

use serenity::client::Context;
use tracing::warn;

pub async fn register_global_commands(ctx: &Context) {
    if let Err(why) = serenity::all::Command::set_global_commands(
        &ctx.http,
        vec![
            voice_controls::register_jam(),
            voice_controls::register_queue(),
            voice_controls::register_skip(),
            voice_controls::register_stop(),
            voice_controls::register_join(),
            stamp::register_stamp(),
        ],
    )
    .await
    {
        warn!("Cannot register global slash commands: {}", why);
    }
}
