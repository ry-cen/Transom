//! The pairing cards' pure transitions (docs/ui-spec.md sections 3.4 and 3.5).

use crate::gui::model::PairFailure;
use crate::gui::texts;
use transom_net::pairing::Paired;
use transom_net::store::SavedMac;
use transom_proto::messages::PairResult;
use transom_proto::pairing::{check_code, CodeProblem};

/// Ok(12 digits) or Err(PAIR5 / PAIR6 text).
pub fn check_typed_code(input: &str) -> Result<String, String> {
    check_code(input).map_err(|p| match p {
        CodeProblem::Length => texts::CODE_LENGTH.to_string(),
        CodeProblem::Typo => texts::CODE_TYPO.to_string(),
    })
}

/// What the Mac's answer to the ask means for the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskOutcome {
    Code { pin: [u8; 32], shown: bool },
    Failed(String),
}

/// "shown" gives `Code { shown: true }` (PAIR3), "openOnMac" gives `Code { shown: false }`
/// (PAIR4); `ok: true`, no reason or any other reason gives a failure text.
pub fn after_ask(mac: &str, r: &Result<(PairResult, [u8; 32]), PairFailure>) -> AskOutcome {
    match r {
        Err(e) => AskOutcome::Failed(texts::pair_failure(mac, e)),
        Ok((res, pin)) => {
            if res.ok {
                return AskOutcome::Failed(texts::no_answer(mac));
            }
            match res.reason.as_deref() {
                Some("shown") => AskOutcome::Code {
                    pin: *pin,
                    shown: true,
                },
                Some("openOnMac") => AskOutcome::Code {
                    pin: *pin,
                    shown: false,
                },
                Some(reason) => AskOutcome::Failed(texts::pair_failure(
                    mac,
                    &PairFailure::Refused {
                        reason: reason.to_string(),
                        tries_left: res.tries_left,
                        retry_after: res.retry_after,
                        message: res.message.clone(),
                    },
                )),
                None => AskOutcome::Failed(texts::no_answer(mac)),
            }
        }
    }
}

/// What the Mac's answer to the code means for the card.
#[derive(Clone, Debug)]
pub enum PairOutcome {
    Paired(SavedMac),
    /// A wrong code with tries left: type it again.
    RetryCode(String),
    Failed(String),
}

pub fn after_pair(mac: &str, r: &Result<Paired, PairFailure>) -> PairOutcome {
    match r {
        Ok(p) => PairOutcome::Paired(p.to_saved()),
        Err(PairFailure::Refused {
            reason,
            tries_left: Some(n),
            ..
        }) if reason == "code" && *n > 0 => PairOutcome::RetryCode(texts::wrong_code(Some(*n))),
        Err(e) => PairOutcome::Failed(texts::pair_failure(mac, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(ok: bool, reason: Option<&str>) -> PairResult {
        PairResult {
            ok,
            reason: reason.map(String::from),
            tries_left: Some(2),
            ..Default::default()
        }
    }

    fn refused(reason: &str, tries: Option<i64>) -> PairFailure {
        PairFailure::Refused {
            reason: reason.into(),
            tries_left: tries,
            retry_after: None,
            message: None,
        }
    }

    #[test]
    fn check_typed_code_texts() {
        assert_eq!(check_typed_code("12"), Err(texts::CODE_LENGTH.to_string()));
        assert_eq!(
            check_typed_code("0000 0000 0001"),
            Err(texts::CODE_TYPO.to_string())
        );
        assert_eq!(
            check_typed_code("0000-0000-0000"),
            Ok("000000000000".into())
        );
    }

    #[test]
    fn after_ask_shown_open_on_mac_and_refusals() {
        let pin = [5u8; 32];
        assert_eq!(
            after_ask("Mac", &Ok((res(false, Some("shown")), pin))),
            AskOutcome::Code { pin, shown: true }
        );
        assert_eq!(
            after_ask("Mac", &Ok((res(false, Some("openOnMac")), pin))),
            AskOutcome::Code { pin, shown: false }
        );
        assert_eq!(
            after_ask("Mac", &Ok((res(true, None), pin))),
            AskOutcome::Failed(texts::no_answer("Mac"))
        );
        assert_eq!(
            after_ask("Mac", &Ok((res(false, Some("closed")), pin))),
            AskOutcome::Failed(texts::pairing_closed("Mac"))
        );
        assert_eq!(
            after_ask("Mac", &Err(PairFailure::Net("x".into()))),
            AskOutcome::Failed(texts::no_answer("Mac"))
        );
    }

    #[test]
    fn after_pair_code_retry_and_failures() {
        let paired = Paired {
            mac_id: "M".into(),
            name: "Mac".into(),
            fingerprint: [7; 32],
            recognition_key: vec![1; 32],
            peer: "127.0.0.1:1".parse().unwrap(),
        };
        match after_pair("Mac", &Ok(paired)) {
            PairOutcome::Paired(s) => assert_eq!(s.mac_id, "M"),
            other => panic!("{other:?}"),
        }
        match after_pair("Mac", &Err(refused("code", Some(2)))) {
            PairOutcome::RetryCode(t) => assert!(t.contains("2 tries left")),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            after_pair("Mac", &Err(refused("code", Some(0)))),
            PairOutcome::Failed(_)
        ));
        assert!(matches!(
            after_pair("Mac", &Err(refused("code", None))),
            PairOutcome::Failed(_)
        ));
        match after_pair("Mac", &Err(refused("expired", None))) {
            PairOutcome::Failed(t) => assert_eq!(t, texts::CODE_EXPIRED),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            after_pair("Mac", &Err(PairFailure::BadProof)),
            PairOutcome::Failed(_)
        ));
    }
}
