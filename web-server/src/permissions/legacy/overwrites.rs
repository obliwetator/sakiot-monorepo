//! The overwrite arithmetic of the old checks.

use super::Permissions;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PermissionOverwriteBits {
    pub(super) allow: Permissions,
    pub(super) deny: Permissions,
}

impl Default for PermissionOverwriteBits {
    fn default() -> Self {
        Self {
            allow: Permissions::empty(),
            deny: Permissions::empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChannelPermissionState {
    pub(super) channel_id: i64,
    pub(super) everyone: PermissionOverwriteBits,
    pub(super) roles: PermissionOverwriteBits,
    pub(super) member: PermissionOverwriteBits,
}

impl ChannelPermissionState {
    pub(super) fn new(channel_id: i64, everyone: PermissionOverwriteBits) -> Self {
        Self {
            channel_id,
            everyone,
            roles: PermissionOverwriteBits::default(),
            member: PermissionOverwriteBits::default(),
        }
    }

    pub(super) fn can_view(self, base_permissions: Permissions) -> bool {
        self.applied(base_permissions)
            .contains(Permissions::VIEW_CHANNEL)
    }

    pub(super) fn can_view_and_connect(self, base_permissions: Permissions) -> bool {
        self.applied(base_permissions)
            .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT)
    }

    fn applied(self, base_permissions: Permissions) -> Permissions {
        let permissions = apply_overwrite(base_permissions, self.everyone);
        let permissions = apply_overwrite(permissions, self.roles);
        apply_overwrite(permissions, self.member)
    }
}

pub(super) fn permissions_from_bits(bits: i64) -> Permissions {
    Permissions::from_bits_retain(bits)
}

fn apply_overwrite(
    mut permissions: Permissions,
    overwrite: PermissionOverwriteBits,
) -> Permissions {
    permissions.remove(overwrite.deny);
    permissions.insert(overwrite.allow);
    permissions
}
