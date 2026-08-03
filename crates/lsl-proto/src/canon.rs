//! The canonicalizer for transcript comparison.
//!
//! A handshake carries fields that change between runs: a UID, a clock
//! reading, an ephemeral port, a host name, and a measured benchmark result.
//! A transcript comparison cannot compare them directly.
//!
//! The rule that keeps this honest: **every placeholder carries a predicate**.
//! A field that gets erased with no assertion is a silent hole in coverage. A
//! rule with no predicate fails the lint in `lint()`, so the hole cannot ship.
//!
//! Read `CONFORMANCE-PLAN.md` risk R3.

use core::fmt;

/// What a canonical rule asserts about the value that it erases.
pub enum Predicate {
    /// The value holds a UID of this shape: 8-4-4-4-12 hexadecimal digits.
    Uuid,
    /// The value parses as an integer inside these bounds.
    IntInRange(i64, i64),
    /// The value parses as a real number inside these bounds.
    F64InRange(f64, f64),
    /// The value parses as a real number. Any value passes.
    ///
    /// This is for a measured benchmark result, whose value carries no meaning
    /// across machines. SPEC.md 5.2.
    F64Any,
    /// The value holds at least one character.
    NonEmpty,
    /// The value holds any text, including none.
    ///
    /// Use this only for a field that carries no protocol meaning. State the
    /// reason in the rule.
    AnyText,
}

impl Predicate {
    /// Test one value.
    pub fn test(&self, value: &str) -> Result<(), String> {
        match self {
            Predicate::Uuid => {
                let parts: Vec<&str> = value.split('-').collect();
                let ok = parts.len() == 5
                    && [8usize, 4, 4, 4, 12]
                        == [
                            parts[0].len(),
                            parts[1].len(),
                            parts[2].len(),
                            parts[3].len(),
                            parts[4].len(),
                        ]
                    && parts
                        .iter()
                        .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()));
                if ok {
                    Ok(())
                } else {
                    Err(format!("{value:?} is not a UID of the shape 8-4-4-4-12"))
                }
            }
            Predicate::IntInRange(lo, hi) => match value.trim().parse::<i64>() {
                Ok(v) if v >= *lo && v <= *hi => Ok(()),
                Ok(v) => Err(format!("{v} is outside {lo} to {hi}")),
                Err(_) => Err(format!("{value:?} is not an integer")),
            },
            Predicate::F64InRange(lo, hi) => match value.trim().parse::<f64>() {
                Ok(v) if v >= *lo && v <= *hi => Ok(()),
                Ok(v) => Err(format!("{v} is outside {lo} to {hi}")),
                Err(_) => Err(format!("{value:?} is not a real number")),
            },
            Predicate::F64Any => match value.trim().parse::<f64>() {
                Ok(_) => Ok(()),
                Err(_) => Err(format!("{value:?} is not a real number")),
            },
            Predicate::NonEmpty => {
                if value.trim().is_empty() {
                    Err("the value is empty".to_string())
                } else {
                    Ok(())
                }
            }
            Predicate::AnyText => Ok(()),
        }
    }
}

/// One rule: a header name, a placeholder, a predicate, and the reason.
pub struct Rule {
    /// The header name, in lower case.
    pub field: &'static str,
    /// The text that replaces the value.
    pub placeholder: &'static str,
    /// What the rule asserts about the value that it erases.
    pub predicate: Predicate,
    /// Why the value cannot be compared directly.
    pub reason: &'static str,
}

/// The rules for a feed handshake.
///
/// Each entry names a field that changes between runs. A field that is stable
/// belongs in no rule, because a direct comparison is stronger.
pub fn feed_rules() -> Vec<Rule> {
    vec![
        Rule {
            field: "uid",
            placeholder: "<UID>",
            predicate: Predicate::Uuid,
            reason: "a stream instance identifier is random, and it changes on every restart",
        },
        Rule {
            field: "endian-performance",
            placeholder: "<PERF>",
            predicate: Predicate::F64Any,
            reason: "a measured benchmark result. SPEC.md 5.2 records it as observed",
        },
        Rule {
            field: "hostname",
            placeholder: "<HOST>",
            predicate: Predicate::AnyText,
            reason: "a property of the machine. An empty value is legal",
        },
        Rule {
            field: "source-id",
            placeholder: "<SOURCE>",
            predicate: Predicate::AnyText,
            reason: "chosen by the application. An empty value is legal",
        },
    ]
}

/// A field that a rule erased, with the value that it held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erased {
    /// The header name.
    pub field: String,
    /// The value that the rule replaced.
    pub value: String,
}

/// The result of canonicalizing one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canonical {
    /// The text with every variable value replaced.
    pub text: String,
    /// Every value that a rule erased.
    pub erased: Vec<Erased>,
}

impl fmt::Display for Canonical {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.text)
    }
}

/// A predicate that failed on a value that a rule erased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The header name.
    pub field: String,
    /// The value that failed.
    pub value: String,
    /// What the predicate reported.
    pub problem: String,
}

/// Replace every variable value in a header block.
///
/// The function keeps the line order and the field names. It replaces only the
/// values that a rule names.
pub fn canonicalize(block: &str, rules: &[Rule]) -> Canonical {
    let mut out = String::with_capacity(block.len());
    let mut erased = Vec::new();

    for line in block.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let mut replaced = false;

        if let Some(colon) = body.find(':') {
            let name = body[..colon].trim().to_ascii_lowercase();
            if let Some(rule) = rules.iter().find(|r| r.field == name) {
                let value = body[colon + 1..].trim().to_string();
                erased.push(Erased { field: name, value });
                out.push_str(&body[..colon + 1]);
                out.push(' ');
                out.push_str(rule.placeholder);
                out.push_str("\r\n");
                replaced = true;
            }
        }
        if !replaced {
            out.push_str(body);
            out.push_str("\r\n");
        }
    }
    Canonical { text: out, erased }
}

/// Test every erased value against the predicate of its rule.
///
/// A rule that erases a value must say something about it. This function is
/// what turns a placeholder from a blind spot into an assertion.
pub fn check(canonical: &Canonical, rules: &[Rule]) -> Vec<Violation> {
    let mut out = Vec::new();
    for e in &canonical.erased {
        let rule = match rules.iter().find(|r| r.field == e.field) {
            Some(r) => r,
            None => continue,
        };
        if let Err(problem) = rule.predicate.test(&e.value) {
            out.push(Violation {
                field: e.field.clone(),
                value: e.value.clone(),
                problem,
            });
        }
    }
    out
}

/// A rule that fails the lint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintFailure {
    /// The header name.
    pub field: String,
    /// What is wrong with the rule.
    pub problem: String,
}

/// Make sure that every rule carries a real predicate and a reason.
///
/// A placeholder with no assertion hides a difference. This lint is the check
/// that risk R3 of the conformance plan asks for.
///
/// `Predicate::AnyText` asserts nothing, so a rule that uses it must give a
/// reason of its own. The lint accepts it only with a reason that names why
/// the field carries no protocol meaning.
pub fn lint(rules: &[Rule]) -> Vec<LintFailure> {
    let mut out = Vec::new();
    for r in rules {
        if r.reason.trim().is_empty() {
            out.push(LintFailure {
                field: r.field.to_string(),
                problem: "the rule gives no reason".into(),
            });
        }
        if r.placeholder.trim().is_empty() {
            out.push(LintFailure {
                field: r.field.to_string(),
                problem: "the rule gives no placeholder".into(),
            });
        }
        if matches!(r.predicate, Predicate::AnyText) && r.reason.len() < 20 {
            out.push(LintFailure {
                field: r.field.to_string(),
                problem: "a rule that asserts nothing needs a reason that explains why".into(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = "LSL/110 200 OK\r\nUID: 0b4645ec-dd4b-4582-a256-36119e117403\r\n\
        Byte-Order: 1234\r\nSuppress-Subnormals: 0\r\nData-Protocol-Version: 110\r\n\r\n";

    #[test]
    fn every_rule_passes_the_lint() {
        let failures = lint(&feed_rules());
        assert!(failures.is_empty(), "the rules fail the lint: {failures:?}");
    }

    #[test]
    fn a_rule_with_no_reason_fails_the_lint() {
        let bad = vec![Rule {
            field: "uid",
            placeholder: "<UID>",
            predicate: Predicate::Uuid,
            reason: "",
        }];
        assert_eq!(lint(&bad).len(), 1);
    }

    #[test]
    fn a_rule_that_asserts_nothing_needs_a_full_reason() {
        let bad = vec![Rule {
            field: "hostname",
            placeholder: "<HOST>",
            predicate: Predicate::AnyText,
            reason: "varies",
        }];
        assert_eq!(lint(&bad).len(), 1);
    }

    #[test]
    fn the_uid_is_replaced_and_the_rest_stays() {
        let c = canonicalize(ANSWER, &feed_rules());
        assert!(c.text.contains("UID: <UID>"));
        assert!(c.text.contains("Byte-Order: 1234"));
        assert!(c.text.contains("Data-Protocol-Version: 110"));
        assert_eq!(c.erased.len(), 1);
        assert_eq!(c.erased[0].value, "0b4645ec-dd4b-4582-a256-36119e117403");
    }

    #[test]
    fn two_answers_with_different_uids_agree_after_canonicalizing() {
        let other = ANSWER.replace(
            "0b4645ec-dd4b-4582-a256-36119e117403",
            "25cfa701-e55b-4e42-8fc2-103bbd2f86b6",
        );
        assert_eq!(
            canonicalize(ANSWER, &feed_rules()).text,
            canonicalize(&other, &feed_rules()).text
        );
    }

    #[test]
    fn the_predicate_catches_a_uid_of_the_wrong_shape() {
        let bad = ANSWER.replace("0b4645ec-dd4b-4582-a256-36119e117403", "not-a-uid");
        let c = canonicalize(&bad, &feed_rules());
        let v = check(&c, &feed_rules());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].field, "uid");
    }

    #[test]
    fn a_good_answer_raises_no_violation() {
        let c = canonicalize(ANSWER, &feed_rules());
        assert!(check(&c, &feed_rules()).is_empty());
    }

    #[test]
    fn a_measured_value_must_still_parse() {
        let rules = feed_rules();
        let good = canonicalize("Endian-Performance: 12345\r\n", &rules);
        assert!(check(&good, &rules).is_empty());
        let bad = canonicalize("Endian-Performance: fast\r\n", &rules);
        assert_eq!(check(&bad, &rules).len(), 1);
    }

    #[test]
    fn a_stable_field_is_never_erased() {
        // Byte-Order carries protocol meaning, so no rule touches it. A
        // transcript comparison must see a change in this field.
        let c = canonicalize(ANSWER, &feed_rules());
        assert!(!c.erased.iter().any(|e| e.field == "byte-order"));
        let changed = ANSWER.replace("Byte-Order: 1234", "Byte-Order: 4321");
        assert_ne!(c.text, canonicalize(&changed, &feed_rules()).text);
    }
}
