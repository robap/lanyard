//! **Am I in a container, and whose?**
//!
//! One detection, shared by the two places that have to suggest a command:
//! [`crate::keys`]'s unwritable-data-directory guardrail and [`crate::doctor`]'s
//! unresolvable-host advice. Two copies would be two chances to tell the same
//! machine to run `docker` and `podman` in the same session.

/// Which container runtime, if any, this `doctor` is running inside.
///
/// **The same detection the unwritable-data-dir guardrail uses**, so the two
/// messages cannot suggest different commands for the same machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    Podman,
    Docker,
    /// Not in a container, so neither `host.*.internal` name is a fact about
    /// this machine — only a suggestion about the other side.
    Host,
}

impl Runtime {
    /// `/run/.containerenv` is podman's; `/.dockerenv` is Docker's.
    pub fn detect() -> Self {
        if std::path::Path::new("/run/.containerenv").exists() {
            Runtime::Podman
        } else if std::path::Path::new("/.dockerenv").exists() {
            Runtime::Docker
        } else {
            Runtime::Host
        }
    }

    /// What this runtime calls the machine the container is on.
    ///
    /// Printed as a **suggestion, never as a fact**: `host.containers.internal`
    /// exists in rootless podman via pasta or slirp4netns, and is not
    /// guaranteed on every configuration.
    pub fn host_alias(&self) -> &'static str {
        match self {
            Runtime::Podman => "host.containers.internal (podman)",
            Runtime::Docker => "host.docker.internal (docker)",
            Runtime::Host => "host.containers.internal (podman) or host.docker.internal (docker)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Podman's name for the host is **not** Docker's, and CONCEPT §8 names only
    /// the Docker spelling — which is the whole reason this distinction exists.
    #[test]
    fn each_runtime_names_the_host_the_way_that_runtime_does() {
        assert!(Runtime::Podman
            .host_alias()
            .contains("host.containers.internal"));
        assert!(!Runtime::Podman
            .host_alias()
            .contains("host.docker.internal"));
        assert!(Runtime::Docker
            .host_alias()
            .contains("host.docker.internal"));
        // On the host neither is a fact about this machine, so both are offered
        // as suggestions about the other side.
        assert!(Runtime::Host
            .host_alias()
            .contains("host.containers.internal"));
        assert!(Runtime::Host.host_alias().contains("host.docker.internal"));
    }

    /// The tests run on a developer machine, not in a container.
    #[test]
    fn detection_reads_the_two_marker_files_and_nothing_else() {
        let detected = Runtime::detect();
        let expected = if std::path::Path::new("/run/.containerenv").exists() {
            Runtime::Podman
        } else if std::path::Path::new("/.dockerenv").exists() {
            Runtime::Docker
        } else {
            Runtime::Host
        };
        assert_eq!(detected, expected);
    }
}
