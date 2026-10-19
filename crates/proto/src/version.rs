//! Port of SillVersion (Compatibility.swift:34-96).

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SillVersion {
    /// The parts, trailing zeros dropped; empty is 0.
    pub components: Vec<u64>,
}

impl SillVersion {
    /// Trims, drops one leading "v"/"V", keeps the longest prefix of ASCII digits and dots
    /// (without trailing dots), which must start with a digit, have at most 8 parts of 1-9
    /// digits each.
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim();
        let t = t
            .strip_prefix('v')
            .or_else(|| t.strip_prefix('V'))
            .unwrap_or(t);
        let end = t
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(t.len());
        let prefix = t[..end].trim_end_matches('.');
        if !prefix.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        let parts: Vec<&str> = prefix.split('.').collect();
        if parts.len() > 8 {
            return None;
        }
        let mut numbers = Vec::with_capacity(parts.len());
        for p in parts {
            if !(1..=9).contains(&p.len()) {
                return None;
            }
            numbers.push(p.parse::<u64>().ok()?);
        }
        Some(Self::from_components(numbers))
    }

    pub fn from_components(mut components: Vec<u64>) -> Self {
        while components.last() == Some(&0) {
            components.pop();
        }
        SillVersion { components }
    }
}

impl Ord for SillVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.components.len().max(other.components.len());
        for i in 0..n {
            let a = self.components.get(i).copied().unwrap_or(0);
            let b = other.components.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for SillVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SillVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut c = self.components.clone();
        while c.len() < 2 {
            c.push(0);
        }
        let parts: Vec<String> = c.iter().map(u64::to_string).collect();
        write!(f, "{}", parts.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SillVersion {
        SillVersion::parse(s).unwrap()
    }

    #[test]
    fn equal_ignoring_zeros_and_v() {
        assert_eq!(v("v0.4.0"), v("0.4"));
    }

    #[test]
    fn numeric_order() {
        assert!(v("0.10") > v("0.9"));
    }

    #[test]
    fn suffix_ignored() {
        assert_eq!(v("1.2.3-beta.1").to_string(), "1.2.3");
    }

    #[test]
    fn invalid() {
        assert_eq!(SillVersion::parse("x1"), None);
        assert_eq!(SillVersion::parse("1..2"), None);
    }

    #[test]
    fn zero_displays_two_parts() {
        assert_eq!(v("0").to_string(), "0.0");
    }
}
