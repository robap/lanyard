//! Scope, and the claim filter it produces.
//!
//! OIDC Core §5.4 specifies claims-per-scope for **the ID token and the UserInfo
//! response**. Nothing specifies it for an access token — an access token is not
//! even required to be a JWT, and its `scope` is a grant for the resource server
//! to read rather than a filter on who the subject is. So the filter is applied
//! where it is specified and nowhere else, which is why
//! [`ClaimFilter::Unfiltered`] exists and why every Phase 2 and Phase 3 caller
//! passes it and is byte-identical to before.

/// The scope that asks for a refresh token, named once so the gate and the
/// discovery document cannot spell it differently.
///
/// It is **not** a claim filter and adds nothing to any token: it appears in
/// `scopes_supported` and in the granted `scope` echoed back, and its only
/// effect is whether `POST /oidc/token` returns a `refresh_token`.
pub const OFFLINE_ACCESS: &str = "offline_access";

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

    /// Whether every scope in `other` was granted here. RFC 6749 §6: a refresh
    /// may **narrow** the grant and may not widen it.
    ///
    /// Returns the first scope that was not granted rather than a bare `false`,
    /// because the refusal has to name it — a developer reading `invalid_scope`
    /// needs to know which of the values they sent was the problem.
    pub fn missing_from<'a>(&self, other: &'a Scopes) -> Option<&'a str> {
        other
            .0
            .iter()
            .find(|scope| !self.has(scope))
            .map(String::as_str)
    }

    /// The space-separated form, which is what a token response's `scope` and a
    /// token's `scope` claim carry. Rebuilt from the parsed set rather than
    /// echoing the request's string, because a narrowed grant is not the string
    /// anybody sent.
    pub fn to_raw(&self) -> String {
        self.0.join(" ")
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

    /// RFC 6749 §6, as a unit test: narrowing is silent, widening names the
    /// offending value.
    #[test]
    fn missing_from_finds_the_scope_that_was_never_granted() {
        let granted = Scopes::parse(Some("openid email offline_access"));
        assert_eq!(granted.missing_from(&Scopes::parse(Some("openid"))), None);
        assert_eq!(granted.missing_from(&Scopes::parse(None)), None);
        assert_eq!(
            granted.missing_from(&Scopes::parse(Some("openid email offline_access"))),
            None,
            "asking for exactly what was granted is not widening"
        );
        assert_eq!(
            granted.missing_from(&Scopes::parse(Some("openid admin"))),
            Some("admin")
        );
    }

    /// The space-separated form, rebuilt rather than echoed: a narrowed grant
    /// is not the string anybody sent.
    #[test]
    fn to_raw_is_the_space_separated_form_of_what_is_held() {
        assert_eq!(
            Scopes::parse(Some("openid  email\tprofile")).to_raw(),
            "openid email profile"
        );
        assert_eq!(Scopes::parse(None).to_raw(), "");
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
