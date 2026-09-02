use anyhow::Context;
use chrono::Utc;

use crate::{
    auth::extractor::AuthClaims,
    model::policy::{AtomicConstraint, Constraint, LogicalConstraint, Policy, RightOperand, Rule},
    policy_engine::{is_part_of, is_subclass_of, lteq},
};

#[derive(Debug)]
pub(crate) enum TransferOutcome {
    Success,
    #[allow(dead_code)]
    Forbidden(String),
}

pub(crate) fn check_transfer_policy(policy: &Policy, claims: &AuthClaims) -> TransferOutcome {
    if !policy
        .permission
        .iter()
        .flatten()
        .all(|r| rule_satisfied(r, claims).unwrap_or(false))
    {
        return TransferOutcome::Forbidden("Permissions not satisifed".into());
    }

    if policy
        .prohibition
        .iter()
        .flatten()
        .any(|r| rule_satisfied(r, claims).unwrap_or(true))
    {
        return TransferOutcome::Forbidden("Prohibitions violated".into());
    }

    TransferOutcome::Success
}

fn rule_satisfied(rule: &Rule, claims: &AuthClaims) -> anyhow::Result<bool> {
    // NOTE: this is an over simplification for demo purposes
    if rule.action != "use" {
        anyhow::bail!("Unsupported action {}", rule.action);
    }

    for constraint in rule.constraint.iter().flatten() {
        if !constraint_satisfied(constraint, claims)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn constraint_satisfied(constraint: &Constraint, claims: &AuthClaims) -> anyhow::Result<bool> {
    match constraint {
        Constraint::Logical(c) => logical_constraint_satisfied(c, claims),
        Constraint::Atomic(c) => atomic_constraint_satisfied(c, claims),
    }
}

fn logical_constraint_satisfied(
    constraint: &LogicalConstraint,
    claims: &AuthClaims,
) -> anyhow::Result<bool> {
    match constraint {
        // NOTE: this is an over simplification for demo purposes as 'and' and 'and_sequence' have to be
        // treated differently
        LogicalConstraint::And { and: list }
        | LogicalConstraint::AndSequence { and_sequence: list } => {
            for c in list.iter() {
                if !constraint_satisfied(c, claims)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        // NOTE: this is an over simplification for demo purposes as 'or' and 'xone' have to be
        // treated differently
        LogicalConstraint::Or { or: list } | LogicalConstraint::Xone { xone: list } => {
            for c in list.iter() {
                if constraint_satisfied(c, claims)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

fn atomic_constraint_satisfied(
    constraint: &AtomicConstraint,
    claims: &AuthClaims,
) -> anyhow::Result<bool> {
    let right_operand = match constraint.left_operand.as_str() {
        "spatial" => claims
            .data
            .get("country") // NOTE: country comes from the mapping between verifiable credentials and claims
            .and_then(|v| v.as_str().map(String::from)),
        "temporal" => Some(Utc::now().to_rfc3339()),
        other => claims
            .data
            .get(other)
            .and_then(|v| v.as_str().map(String::from)),
    }
    .map(RightOperand::String);

    let right_operand = right_operand.context("Unsufficient claims")?;

    Ok(match constraint.operator {
        crate::model::policy::Operator::Eq => constraint.right_operand == right_operand,
        crate::model::policy::Operator::Neq => constraint.right_operand != right_operand,
        crate::model::policy::Operator::Gt => lteq(
            &constraint.left_operand,
            &constraint.right_operand,
            &right_operand,
            false,
        ),
        crate::model::policy::Operator::Gteq => lteq(
            &constraint.left_operand,
            &constraint.right_operand,
            &right_operand,
            true,
        ),
        crate::model::policy::Operator::Lt => lteq(
            &constraint.left_operand,
            &right_operand,
            &constraint.right_operand,
            false,
        ),
        crate::model::policy::Operator::Lteq => lteq(
            &constraint.left_operand,
            &right_operand,
            &constraint.right_operand,
            true,
        ),
        crate::model::policy::Operator::HasPart => is_part_of(
            &constraint.left_operand,
            &constraint.right_operand,
            &right_operand,
        ),
        crate::model::policy::Operator::IsPartOf => is_part_of(
            &constraint.left_operand,
            &right_operand,
            &constraint.right_operand,
        ),
        crate::model::policy::Operator::IsA => is_subclass_of(
            &constraint.left_operand,
            &right_operand,
            &constraint.right_operand,
        ),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use serde_json::Value;

    use crate::{
        auth::extractor::AuthClaims,
        model::policy::{
            AtomicConstraint, Constraint, LogicalConstraint, Operator, Policy, RightOperand, Rule,
        },
        policy_engine::transfer::{TransferOutcome, check_transfer_policy},
    };

    #[test]
    fn test_temporal() {
        let now = Utc::now();
        let policy = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(30)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };

        let claims = AuthClaims::default();
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Success
        ));
    }

    #[test]
    fn test_spatial() {
        let policy = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "spatial".into(),
                    operator: Operator::IsPartOf,
                    right_operand: RightOperand::String("_:EU".into()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };

        let claims = AuthClaims::default();
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Forbidden(_)
        ));

        let mut claims = AuthClaims::default();
        claims
            .data
            .insert("country".into(), Value::String("DE".into()));
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Success
        ));

        let mut claims = AuthClaims::default();
        claims
            .data
            .insert("country".into(), Value::String("US".into()));
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Forbidden(_)
        ));
    }

    #[test]
    fn test_logical() {
        let now = Utc::now();
        let policy = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Logical(LogicalConstraint::And {
                    and: vec![
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "temporal".into(),
                            operator: Operator::Lteq,
                            right_operand: RightOperand::String(
                                (now + Duration::days(30)).to_rfc3339(),
                            ),
                        }),
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "spatial".into(),
                            operator: Operator::IsPartOf,
                            right_operand: RightOperand::String("_:EU".into()),
                        }),
                    ],
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };

        let claims = AuthClaims::default();
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Forbidden(_)
        ));

        let mut claims = AuthClaims::default();
        claims
            .data
            .insert("country".into(), Value::String("DE".into()));
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Success
        ));

        let mut claims = AuthClaims::default();
        claims
            .data
            .insert("country".into(), Value::String("US".into()));
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Forbidden(_)
        ));
    }

    #[test]
    fn test_prohibition() {
        let now = Utc::now();
        let policy = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(30)).to_rfc3339()),
                })]),
            }]),
            prohibition: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "spatial".into(),
                    operator: Operator::IsPartOf,
                    right_operand: RightOperand::String("_:EU".into()),
                })]),
            }]),
            obligation: None,
        };

        let claims = AuthClaims::default();
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Forbidden(_)
        ));

        let mut claims = AuthClaims::default();
        claims
            .data
            .insert("country".into(), Value::String("DE".into()));
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Forbidden(_)
        ));

        let mut claims = AuthClaims::default();
        claims
            .data
            .insert("country".into(), Value::String("UK".into()));
        assert!(matches!(
            check_transfer_policy(&policy, &claims),
            TransferOutcome::Success
        ));
    }
}
