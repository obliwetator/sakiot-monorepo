//! Capabilities by name, as the API shows them.

use sakiot_permissions::Capabilities;

/// One of Sakiot's capabilities. Guild-wide ones: `manage_settings`
/// (cooldowns, recording policy, voice settings), `manage_permissions`,
/// `preview_roles` (role preview and the roles page), `delete_recordings`
/// and `moderate_clips` (someone else's clip). Per channel: `view_presence`,
/// `listen_others`, `create_clips`, `jam` and `stamp`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ManageSettings,
    ManagePermissions,
    PreviewRoles,
    DeleteRecordings,
    ModerateClips,
    ViewPresence,
    ListenOthers,
    CreateClips,
    Jam,
    Stamp,
}

const NAMES: [(Capability, Capabilities); 10] = [
    (Capability::ManageSettings, Capabilities::MANAGE_SETTINGS),
    (
        Capability::ManagePermissions,
        Capabilities::MANAGE_PERMISSIONS,
    ),
    (Capability::PreviewRoles, Capabilities::PREVIEW_ROLES),
    (
        Capability::DeleteRecordings,
        Capabilities::DELETE_RECORDINGS,
    ),
    (Capability::ModerateClips, Capabilities::MODERATE_CLIPS),
    (Capability::ViewPresence, Capabilities::VIEW_PRESENCE),
    (Capability::ListenOthers, Capabilities::LISTEN_OTHERS),
    (Capability::CreateClips, Capabilities::CREATE_CLIPS),
    (Capability::Jam, Capabilities::JAM),
    (Capability::Stamp, Capabilities::STAMP),
];

impl Capability {
    /// The capabilities in `set`, by name.
    pub fn named(set: Capabilities) -> Vec<Self> {
        NAMES
            .iter()
            .filter(|(_, bit)| set.contains(*bit))
            .map(|&(name, _)| name)
            .collect()
    }

    /// The name of one capability; `None` for any other set.
    pub fn one(capability: Capabilities) -> Option<Self> {
        NAMES
            .iter()
            .find(|(_, bit)| *bit == capability)
            .map(|&(name, _)| name)
    }

    /// The capabilities `names` name.
    pub fn set(names: &[Self]) -> Capabilities {
        names.iter().fold(Capabilities::empty(), |set, name| {
            NAMES
                .iter()
                .find(|(named, _)| named == name)
                .map_or(set, |&(_, bit)| set | bit)
        })
    }

    /// The name as the API spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManageSettings => "manage_settings",
            Self::ManagePermissions => "manage_permissions",
            Self::PreviewRoles => "preview_roles",
            Self::DeleteRecordings => "delete_recordings",
            Self::ModerateClips => "moderate_clips",
            Self::ViewPresence => "view_presence",
            Self::ListenOthers => "listen_others",
            Self::CreateClips => "create_clips",
            Self::Jam => "jam",
            Self::Stamp => "stamp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_has_one_name() {
        let named = NAMES
            .iter()
            .try_fold(Capabilities::empty(), |named, &(_, bit)| {
                (bit.bits().count_ones() == 1 && !named.contains(bit)).then_some(named | bit)
            });
        assert_eq!(named, Some(Capabilities::all()));
        for &(name, bit) in &NAMES {
            assert_eq!(Capability::one(bit), Some(name));
            assert_eq!(Capability::set(&[name]), bit);
            assert_eq!(
                serde_json::to_value(name).ok(),
                Some(serde_json::Value::from(name.as_str()))
            );
        }
        assert_eq!(Capability::one(Capabilities::MANAGEMENT), None);
        assert_eq!(
            Capability::named(Capabilities::MANAGEMENT),
            [
                Capability::ManageSettings,
                Capability::ManagePermissions,
                Capability::PreviewRoles,
                Capability::DeleteRecordings,
                Capability::ModerateClips,
            ]
        );
    }
}
