use std::cmp::Ordering;

/// Represents a parsed version string for semantic comparison.
#[derive(Debug, Clone)]
pub struct Version {
    #[allow(dead_code)]
    pub raw: String,
    pub segments: Vec<u64>,
    pub prerelease: Option<String>,
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl Version {
    /// Parses a version string into segments and optional pre-release identifier.
    /// Handles prefixes like 'v', 'V', 'release-', and dots/hyphens.
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        let stripped = if let Some(p) = trimmed.strip_prefix("release-") {
            p
        } else if trimmed.starts_with('v') || trimmed.starts_with('V') {
            &trimmed[1..]
        } else {
            trimmed
        };

        // Split into version core and pre-release suffix (e.g. "1.2.3-rc1" or "1.2.3_beta")
        let (core, prerelease) = if let Some(pos) = stripped.find(['-', '_']) {
            (&stripped[..pos], Some(stripped[pos + 1..].to_string()))
        } else {
            (stripped, None)
        };

        let mut segments = Vec::new();
        for chunk in core.split('.') {
            // Take digits from the beginning of each chunk
            let digits: String = chunk.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(num) = digits.parse::<u64>() {
                segments.push(num);
            }
        }

        Self {
            raw: s.to_string(),
            segments,
            prerelease,
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let max_len = self.segments.len().max(other.segments.len());
        for i in 0..max_len {
            let a = self.segments.get(i).copied().unwrap_or(0);
            let b = other.segments.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => continue,
                non_eq => return non_eq,
            }
        }

        // Both segment sets equal: compare pre-release
        // A release without pre-release tag is NEWER than one with a pre-release tag
        // e.g. 1.0.0 > 1.0.0-rc1
        match (&self.prerelease, &other.prerelease) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionComparison {
    UpToDate,
    UpdateAvailable,
    AheadOfUpstream,
    NotInstalled,
}

/// Compares a local version string against an upstream remote version string.
pub fn compare(local: &str, remote: &str) -> VersionComparison {
    if local.is_empty() || local.eq_ignore_ascii_case("not installed") {
        return VersionComparison::NotInstalled;
    }

    let local_ver = Version::parse(local);
    let remote_ver = Version::parse(remote);

    match remote_ver.cmp(&local_ver) {
        Ordering::Greater => VersionComparison::UpdateAvailable,
        Ordering::Equal => VersionComparison::UpToDate,
        Ordering::Less => VersionComparison::AheadOfUpstream,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_ordering() {
        let v1 = Version::parse("5.0.0");
        let v2 = Version::parse("5.0.1");
        assert!(v2 > v1);

        let v_local = Version::parse("v1.2.10");
        let v_remote = Version::parse("1.2.9");
        assert!(v_local > v_remote);

        let v_equal1 = Version::parse("v2.3.0");
        let v_equal2 = Version::parse("2.3.0");
        assert_eq!(v_equal1, v_equal2);

        let v_release = Version::parse("4.0.0");
        let v_rc = Version::parse("4.0.0-rc1");
        assert!(v_release > v_rc);
    }

    #[test]
    fn test_comparison_enum() {
        assert_eq!(compare("Not Installed", "1.0.0"), VersionComparison::NotInstalled);
        assert_eq!(compare("", "1.0.0"), VersionComparison::NotInstalled);
        assert_eq!(compare("1.0.0", "1.0.1"), VersionComparison::UpdateAvailable);
        assert_eq!(compare("1.0.1", "1.0.1"), VersionComparison::UpToDate);
        assert_eq!(compare("v1.0.2", "1.0.1"), VersionComparison::AheadOfUpstream);
    }
}
