//! Who may change a Sakiot rule, and how far.

use crate::access::MembershipUnavailable;
use crate::capabilities::Capabilities;
use crate::facts::{GuildFacts, Viewer};
use crate::rules::{Rule, Subject};

/// Why a rule change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleRefused {
    /// Whether the editor belongs is not known yet. Retryable.
    MembershipUnavailable,
    /// The editor does not hold `MANAGE_PERMISSIONS`.
    NotAnEditor,
    /// The rule allows and denies the same capability.
    AllowsAndDenies,
    /// A channel rule names a management capability, which only applies
    /// guild-wide.
    ManagementInChannel,
    /// The change touches capabilities the editor does not hold where the
    /// rule applies (`Access::editable`).
    NotHeld(Capabilities),
    /// The change would take `MANAGE_PERMISSIONS` from the editor.
    OwnManagePermissions,
}

impl From<MembershipUnavailable> for RuleRefused {
    fn from(MembershipUnavailable: MembershipUnavailable) -> Self {
        Self::MembershipUnavailable
    }
}

impl GuildFacts {
    /// Sets `editor`'s rule for `subject`, guild-wide (`channel` `None`)
    /// or in `channel`, if they may; an empty rule removes it. The facts must
    /// hold the editor, and the subject when it is a role or a user, so the
    /// rule it replaces is loaded.
    ///
    /// The editor must hold `MANAGE_PERMISSIONS`, may change only what they
    /// hold where the rule applies, and may not take `MANAGE_PERMISSIONS`
    /// from themselves. The owner always keeps it, so these never stop the
    /// owner.
    pub fn edit_rule(
        &mut self,
        editor: Viewer,
        channel: Option<i64>,
        subject: Subject,
        rule: Rule,
    ) -> Result<(), RuleRefused> {
        let before = self.access(editor)?;
        if !before.guild.contains(Capabilities::MANAGE_PERMISSIONS) {
            return Err(RuleRefused::NotAnEditor);
        }
        if rule.allow.intersects(rule.deny) {
            return Err(RuleRefused::AllowsAndDenies);
        }
        if channel.is_some() && rule.mentions().intersects(Capabilities::MANAGEMENT) {
            return Err(RuleRefused::ManagementInChannel);
        }
        let old = self.rules.get(channel, subject);
        // Management capabilities in a channel rule do nothing, so neither
        // does changing them.
        let applies = if channel.is_some() {
            Capabilities::CONTENT
        } else {
            Capabilities::all()
        };
        let changed = ((old.allow ^ rule.allow) | (old.deny ^ rule.deny)) & applies;
        // A rule in a channel that is gone reaches nobody; anyone may tidy
        // it away.
        let reaches_nobody = channel.is_some_and(|channel| !self.channels.contains_key(&channel));
        let beyond = changed - before.editable(channel);
        if !reaches_nobody && !beyond.is_empty() {
            return Err(RuleRefused::NotHeld(beyond));
        }
        self.rules.set(channel, subject, rule);
        let after = self.access(editor)?;
        if !after.guild.contains(Capabilities::MANAGE_PERMISSIONS) {
            self.rules.set(channel, subject, old);
            return Err(RuleRefused::OwnManagePermissions);
        }
        Ok(())
    }
}
