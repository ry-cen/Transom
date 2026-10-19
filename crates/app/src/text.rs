//! User-facing texts.

use transom_net::NetError;
use transom_proto::safetext;

/// The text for a refused pairing, by the Mac's reason.
pub fn pair_refusal(
    reason: &str,
    tries_left: Option<i64>,
    retry_after: Option<f64>,
    message: Option<&str>,
) -> String {
    match reason {
        "code" => match tries_left {
            Some(n) => format!("Wrong code. {n} tries left."),
            None => "Wrong code.".to_string(),
        },
        "closed" => "The Mac's pairing window is closed. On the Mac, choose Pair iPhone or iPad… in Sill's menu, then try again.".to_string(),
        "expired" => "The code expired. Open the pairing window on the Mac again.".to_string(),
        "stopped" => "The Mac stopped pairing.".to_string(),
        "busy" => {
            let secs = retry_after.filter(|s| s.is_finite()).unwrap_or(0.0).max(0.0).ceil();
            format!("The Mac is busy; try again in {secs:.0} s.")
        }
        "shown" => "The Mac shows a pairing code now.".to_string(),
        "openOnMac" => "On the Mac, choose Pair iPhone or iPad… in Sill's menu; it shows a code.".to_string(),
        "locked" => "The Mac is locked. Unlock it and try again.".to_string(),
        other => {
            let shown = message
                .map(|m| safetext::label(m, 300))
                .unwrap_or_default();
            if shown.is_empty() {
                format!("The Mac refused: {}.", safetext::label(other, 64))
            } else {
                shown
            }
        }
    }
}

pub fn net_error(e: &NetError) -> String {
    match e {
        NetError::NotPaired => {
            "This computer is not paired with that Mac. Run: transom pair".to_string()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_refusal_texts() {
        for r in [
            "code",
            "closed",
            "expired",
            "stopped",
            "busy",
            "shown",
            "openOnMac",
            "locked",
            "mystery",
        ] {
            assert!(!pair_refusal(r, None, None, None).is_empty(), "{r}");
        }
        assert_eq!(
            pair_refusal("busy", None, Some(2.1), None),
            "The Mac is busy; try again in 3 s."
        );
        assert_eq!(
            pair_refusal("code", Some(2), None, None),
            "Wrong code. 2 tries left."
        );
        assert_eq!(
            pair_refusal("mystery", None, None, Some("  hello\n there ")),
            "hello there"
        );
        assert_eq!(
            pair_refusal("mystery", None, None, None),
            "The Mac refused: mystery."
        );
    }

    #[test]
    fn not_paired_text() {
        assert!(net_error(&NetError::NotPaired).contains("transom pair"));
    }
}
