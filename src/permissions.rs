//! Authorization model: **tiers** (what the server stores per user) built from
//! **permissions** (the granular capability vocabulary).
//!
//! The server intentionally ships only a fixed **tier** system — every user has
//! exactly one [`Tier`] (`user` / `moderator` / `admin`) — while authorization
//! is *checked* against [`Permission`]s. The tier→permission mapping lives in
//! one place ([`Tier::allows`]); a downstream deployment that wants a different
//! policy (more tiers, different bundles, a DB-driven role system) only has to
//! re-map permissions here, without touching every call site.
//!
//! Authorization at a call site is always `user.tier().allows(Permission::X)`
//! (composed, where relevant, with the token scope and per-resource ownership).

/// A granular capability. Handlers check these, never tiers directly, so the
/// policy can change in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// View the moderation report queue.
    ViewReports,
    /// Resolve or dismiss reports.
    ResolveReports,
    /// Yank/unyank any package version, regardless of ownership.
    YankAnyPackage,
    /// View the registry-wide admin overview.
    ViewOverview,
    /// Manage user accounts (roles, removal).
    ManageUsers,
    /// Hard-delete any package.
    DeleteAnyPackage,
    /// Registry-wide configuration / destructive maintenance.
    ManageRegistry,
}

/// A user's role tier. This is the only authorization state the server persists
/// per user. Ordered: `User` < `Moderator` < `Admin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    User,
    Moderator,
    Admin,
}

impl Tier {
    /// All tiers the server ships, lowest-privilege first.
    pub const ALL: [Tier; 3] = [Tier::User, Tier::Moderator, Tier::Admin];

    /// Parse a stored role string. Unknown/empty values are **not** matched
    /// (callers fall back to legacy `is_admin`); see [`crate::db::UserRow::tier`].
    pub fn from_role(s: &str) -> Option<Tier> {
        match s {
            "user" => Some(Tier::User),
            "moderator" => Some(Tier::Moderator),
            "admin" => Some(Tier::Admin),
            _ => None,
        }
    }

    /// The canonical string stored in the `users.role` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::User => "user",
            Tier::Moderator => "moderator",
            Tier::Admin => "admin",
        }
    }

    /// Privilege rank (higher = more). Useful for "at least moderator" checks.
    pub fn rank(self) -> u8 {
        match self {
            Tier::User => 0,
            Tier::Moderator => 1,
            Tier::Admin => 2,
        }
    }

    /// **The policy.** Whether this tier is granted a permission. Edit this one
    /// function to change the authorization policy for the whole server.
    pub fn allows(self, perm: Permission) -> bool {
        use Permission::*;
        match self {
            Tier::Admin => true, // admins hold every permission
            Tier::Moderator => matches!(
                perm,
                ViewReports | ResolveReports | YankAnyPackage | ViewOverview
            ),
            Tier::User => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_string_roundtrip() {
        for t in Tier::ALL {
            assert_eq!(Tier::from_role(t.as_str()), Some(t));
        }
        assert_eq!(Tier::from_role("nonsense"), None);
        assert_eq!(Tier::from_role(""), None);
    }

    #[test]
    fn moderator_can_moderate_but_not_manage_users() {
        let m = Tier::Moderator;
        assert!(m.allows(Permission::ViewReports));
        assert!(m.allows(Permission::ResolveReports));
        assert!(m.allows(Permission::YankAnyPackage));
        assert!(m.allows(Permission::ViewOverview));
        assert!(!m.allows(Permission::ManageUsers));
        assert!(!m.allows(Permission::DeleteAnyPackage));
        assert!(!m.allows(Permission::ManageRegistry));
    }

    #[test]
    fn user_has_no_elevated_permissions() {
        let u = Tier::User;
        for p in [
            Permission::ViewReports,
            Permission::ResolveReports,
            Permission::YankAnyPackage,
            Permission::ViewOverview,
            Permission::ManageUsers,
            Permission::DeleteAnyPackage,
            Permission::ManageRegistry,
        ] {
            assert!(!u.allows(p));
        }
    }

    #[test]
    fn admin_holds_everything() {
        let a = Tier::Admin;
        for p in [
            Permission::ViewReports,
            Permission::ResolveReports,
            Permission::YankAnyPackage,
            Permission::ViewOverview,
            Permission::ManageUsers,
            Permission::DeleteAnyPackage,
            Permission::ManageRegistry,
        ] {
            assert!(a.allows(p));
        }
        assert!(a.rank() > Tier::Moderator.rank());
        assert!(Tier::Moderator.rank() > Tier::User.rank());
    }
}
