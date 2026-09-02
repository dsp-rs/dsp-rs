use chrono::DateTime;

use crate::model::policy::RightOperand;

#[cfg(not(feature = "tck"))]
pub(crate) mod negotiation;

pub(crate) mod transfer;

pub(super) fn is_subclass_of(_property: &str, a: &RightOperand, b: &RightOperand) -> bool {
    let (RightOperand::String(a), RightOperand::String(b)) = (a, b) else {
        return false;
    };

    // NOTE: currently no subclass subsumption supported
    a == b
}

pub(super) fn is_part_of(property: &str, a: &RightOperand, b: &RightOperand) -> bool {
    // NOTE: this is an over simplification for demo purposes
    match property {
        "spatial" => {
            let (RightOperand::String(a), RightOperand::String(b)) = (a, b) else {
                return a == b;
            };
            match (a.to_lowercase().as_str(), b.to_lowercase().as_str()) {
                (
                    "at" | "be" | "bg" | "hr" | "cy" | "cz" | "dk" | "ee" | "fi" | "fr" | "de"
                    | "gr" | "hu" | "ie" | "it" | "lv" | "lt" | "lu" | "mt" | "nl" | "pl" | "pt"
                    | "ro" | "sk" | "si" | "es" | "se",
                    "_:eu",
                ) => true,
                _ => a == b, // e.g. "de is part of de"
            }
        }
        _ => false,
    }
}

pub(super) fn lteq(property: &str, a: &RightOperand, b: &RightOperand, eq: bool) -> bool {
    // TODO: add support for JSON-LD encoded typed literals,
    // e.g. {"@value": 42, "@type": "http://www.w3.org/2001/XMLSchema#integer"}

    let (RightOperand::String(a), RightOperand::String(b)) = (a, b) else {
        return false;
    };

    // NOTE: this is an over simplification for demo purposes
    match property {
        "temporal" => {
            // try to parse as "http://www.w3.org/2001/XMLSchema#dateTime"
            match (
                DateTime::parse_from_rfc3339(a),
                DateTime::parse_from_rfc3339(b),
            ) {
                (Ok(a), Ok(b)) => {
                    if eq {
                        a <= b
                    } else {
                        a < b
                    }
                }
                _ => false,
            }
        }
        _ => false,
    }
}
