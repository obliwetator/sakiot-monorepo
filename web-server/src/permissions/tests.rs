use super::*;

fn state(
    everyone: PermissionOverwriteBits,
    roles: PermissionOverwriteBits,
    member: PermissionOverwriteBits,
) -> ChannelPermissionState {
    ChannelPermissionState {
        channel_id: 1,
        everyone,
        roles,
        member,
    }
}

#[test]
fn everyone_deny_blocks_base_connect() {
    let channel = state(
        PermissionOverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::CONNECT,
        },
        PermissionOverwriteBits::default(),
        PermissionOverwriteBits::default(),
    );

    assert!(!channel.can_view_and_connect(Permissions::VIEW_CHANNEL | Permissions::CONNECT));
}

#[test]
fn role_allow_restores_everyone_deny() {
    let channel = state(
        PermissionOverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::CONNECT,
        },
        PermissionOverwriteBits {
            allow: Permissions::CONNECT,
            deny: Permissions::empty(),
        },
        PermissionOverwriteBits::default(),
    );

    assert!(channel.can_view_and_connect(Permissions::VIEW_CHANNEL));
}

#[test]
fn member_deny_overrides_role_allow() {
    let channel = state(
        PermissionOverwriteBits::default(),
        PermissionOverwriteBits {
            allow: Permissions::CONNECT,
            deny: Permissions::empty(),
        },
        PermissionOverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::CONNECT,
        },
    );

    assert!(!channel.can_view_and_connect(Permissions::VIEW_CHANNEL));
}

#[test]
fn member_allow_overrides_role_deny() {
    let channel = state(
        PermissionOverwriteBits::default(),
        PermissionOverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::CONNECT,
        },
        PermissionOverwriteBits {
            allow: Permissions::CONNECT,
            deny: Permissions::empty(),
        },
    );

    assert!(channel.can_view_and_connect(Permissions::VIEW_CHANNEL));
}

#[test]
fn view_channel_deny_blocks_inherited_connect() {
    let channel = state(
        PermissionOverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL,
        },
        PermissionOverwriteBits::default(),
        PermissionOverwriteBits::default(),
    );

    assert!(!channel.can_view_and_connect(Permissions::VIEW_CHANNEL | Permissions::CONNECT));
}

#[test]
fn unknown_permission_bits_are_retained() {
    let future_discord_permission = 1_i64 << 50;

    assert_eq!(
        permissions_from_bits(future_discord_permission).bits(),
        future_discord_permission
    );
}
