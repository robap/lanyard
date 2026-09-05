//! Scope, and the claim filter it produces.
//!
//! OIDC Core §5.4 specifies claims-per-scope for **the ID token and the UserInfo
//! response**. Nothing specifies it for an access token — an access token is not
//! even required to be a JWT, and its `scope` is a grant for the resource server
//! to read rather than a filter on who the subject is. So the filter is applied
//! where it is specified and nowhere else, which is why
//! [`ClaimFilter::Unfiltered`] exists and why every Phase 2 and Phase 3 caller
//! passes it and is byte-identical to before.

/// The scopes an authorization request asked for, split on whitespace as RFC
/// 6749 §3.3 defines them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scopes(Vec<String>);

impl Scopes {
    pub fn parse(raw: Option<&str>) -> Self {
        Scopes(
            raw.unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect(),
        )
    }

    pub fn has(&self, scope: &str) -> bool {
        self.0.iter().any(|s| s == scope)
    }

    /// Whether an ID token is owed at all. Without it this is a plain OAuth 2.0
    /// code flow and gets a plain OAuth 2.0 response, which is honest and worth
    /// being able to test.
    pub fn is_openid(&self) -> bool {
        self.has("openid")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Which persona claims survive into a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimFilter {
    /// Everything the persona has. Access tokens and the test seam.
    Unfiltered,
    /// OIDC Core §5.4's table. ID tokens and `/userinfo`.
    ByScope(Scopes),
}

impl ClaimFilter {
    /// `sub`, `roles` and the persona's arbitrary `attributes` are always
    /// allowed: OIDC defines no scope for them, and hiding a developer's own
    /// custom claims behind a standard scope would be inventing a rule.
    pub fn allows(&self, claim: &str) -> bool {
        let scopes = match self {
            ClaimFilter::Unfiltered => return true,
            ClaimFilter::ByScope(scopes) => scopes,
        };
        match claim {
            "email" | "email_verified" => scopes.has("email"),
            "name" | "preferred_username" => scopes.has("profile"),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by(raw: &str) -> ClaimFilter {
        ClaimFilter::ByScope(Scopes::parse(Some(raw)))
    }

    #[test]
    fn scopes_split_on_any_whitespace_and_survive_being_absent() {
        assert!(Scopes::parse(None).is_empty());
        assert!(Scopes::parse(Some("   ")).is_empty());
        let s = Scopes::parse(Some("openid  email\tprofile"));
        assert!(s.has("openid") && s.has("email") && s.has("profile"));
        assert!(!s.has("openi"), "membership is exact, not a prefix match");
    }

    #[test]
    fn unfiltered_allows_every_claim_including_ones_nobody_named() {
        let f = ClaimFilter::Unfiltered;
        for claim in [
            "sub",
            "email",
            "email_verified",
            "name",
            "preferred_username",
            "roles",
        ] {
            assert!(f.allows(claim), "{claim} must survive an unfiltered mint");
        }
    }

    #[test]
    fn email_scope_gates_the_two_email_claims() {
        assert!(!by("openid").allows("email"));
        assert!(!by("openid").allows("email_verified"));
        assert!(by("openid email").allows("email"));
        assert!(by("openid email").allows("email_verified"));
    }

    #[test]
    fn profile_scope_gates_the_display_identity() {
        assert!(!by("openid email").allows("name"));
        assert!(!by("openid email").allows("preferred_username"));
        assert!(by("openid profile").allows("name"));
        assert!(by("openid profile").allows("preferred_username"));
    }

    /// The asymmetry stated as a test: OIDC defines no scope for these, so
    /// gating them behind one would be lanyard inventing a rule.
    #[test]
    fn sub_roles_and_arbitrary_attributes_need_no_scope() {
        let bare = by("openid");
        assert!(bare.allows("sub"));
        assert!(bare.allows("roles"));
        assert!(bare.allows("department"), "a persona's own attribute");
    }

    /// A request with no `openid` still filters — it just gets no ID token to
    /// filter *into*. The filter and the decision are separate questions.
    #[test]
    fn openid_decides_whether_a_token_is_owed_not_which_claims_it_carries() {
        assert!(!Scopes::parse(Some("profile email")).is_openid());
        assert!(Scopes::parse(Some("openid")).is_openid());
        assert!(by("profile email").allows("email"));
    }
}
