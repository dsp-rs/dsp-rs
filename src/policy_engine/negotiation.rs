use crate::{
    model::policy::{AtomicConstraint, Constraint, LogicalConstraint, Operator, Policy, Rule},
    policy_engine::{is_part_of, is_subclass_of, lteq},
};

#[derive(Debug)]
pub(crate) enum NegotiationOutcome {
    Success(Policy),
    #[allow(dead_code)]
    Forbidden(String),
}

pub(crate) fn negotiate_policy(required: &Policy, offered: &Policy) -> NegotiationOutcome {
    let mut negotiated_permissions = Vec::new();
    let required_perms = required.permission.as_ref().into_iter().flatten();

    // all required permissions should be covered by the offer; offer permissions
    // might be more restrictive than corresponding required ones
    for r_perm in required_perms {
        let matched_perm = offered
            .permission
            .as_ref()
            .into_iter()
            .flatten()
            .find(|o_perm| {
                r_perm.action == o_perm.action // NOTE: no subsumption for actions, i.e. "use
                                               // implies read and write" is not supported
                    && constraints_are_compatible(&o_perm.constraint, &r_perm.constraint)
            });

        match matched_perm {
            Some(m_perm) => {
                negotiated_permissions.push(Rule {
                    action: m_perm.action.clone(),
                    constraint: m_perm.constraint.clone(),
                });
            }
            None => {
                return NegotiationOutcome::Forbidden(format!(
                    "The offer's policy does not satisfy the permission requirement for action '{}'",
                    r_perm.action
                ));
            }
        }
    }

    // all consumer permissions are checked against the provider prohibitions
    let offered_perms = offered.permission.as_ref().into_iter().flatten();

    for o_perm in offered_perms {
        let matched_prohib = required
            .prohibition
            .as_ref()
            .into_iter()
            .flatten()
            .find(|prohib| {
                o_perm.action == prohib.action
                    && constraints_are_compatible(&o_perm.constraint, &prohib.constraint)
            });
        if let Some(prohib) = matched_prohib {
            return NegotiationOutcome::Forbidden(format!(
                "Requested action '{}' collides with prohibition",
                prohib.action
            ));
        }
    }

    // all obligations should be present in the offer, i.e. confirmed by the consumer
    let obligations = required.obligation.as_ref().into_iter().flatten();

    for p_ob in obligations {
        let explicitly_matched = offered
            .obligation
            .as_ref()
            .into_iter()
            .flatten()
            .any(|c_ob| {
                c_ob.action == p_ob.action
                    && constraints_are_identical(&c_ob.constraint, &p_ob.constraint)
            });

        if !explicitly_matched {
            return NegotiationOutcome::Forbidden(format!(
                "Required obligation '{}' was not explicitly matched by the offer",
                p_ob.action
            ));
        }
    }

    NegotiationOutcome::Success(Policy {
        profile: required.profile.clone(),
        permission: if negotiated_permissions.is_empty() {
            None
        } else {
            Some(negotiated_permissions)
        },
        prohibition: required.prohibition.clone(),
        obligation: required.obligation.clone(),
    })
}

fn constraints_are_compatible(a: &Option<Vec<Constraint>>, b: &Option<Vec<Constraint>>) -> bool {
    match (a, b) {
        (_, None) => true,
        (None, Some(p)) if p.is_empty() => true,
        (None, Some(_)) => false,
        (Some(c), Some(p)) if c.is_empty() && !p.is_empty() => false,

        (Some(a_constraints), Some(b_constraints)) => b_constraints.iter().all(|bc| {
            a_constraints
                .iter()
                .any(|ac| evaluate_structural_compatibility(ac, bc))
        }),
    }
}

fn evaluate_structural_compatibility(a: &Constraint, b: &Constraint) -> bool {
    match (a, b) {
        (Constraint::Atomic(a_atom), Constraint::Atomic(b_atom)) => subsumed_by(a_atom, b_atom),

        (Constraint::Logical(a_log), Constraint::Logical(b_log)) => match (a_log, b_log) {
            (LogicalConstraint::And { and: a_list }, LogicalConstraint::And { and: b_list })
            | (
                LogicalConstraint::AndSequence {
                    and_sequence: a_list,
                },
                LogicalConstraint::AndSequence {
                    and_sequence: b_list,
                },
            ) => b_list.iter().all(|bb| {
                a_list
                    .iter()
                    .any(|aa| evaluate_structural_compatibility(aa, bb))
            }),
            (LogicalConstraint::Or { or: a_list }, LogicalConstraint::Or { or: b_list })
            | (
                LogicalConstraint::Xone { xone: a_list },
                LogicalConstraint::Xone { xone: b_list },
            ) => a_list.iter().any(|aa| {
                b_list
                    .iter()
                    .any(|bb| evaluate_structural_compatibility(aa, bb))
            }),
            _ => false,
        },

        (Constraint::Atomic(_), Constraint::Logical(b_log)) => match b_log {
            LogicalConstraint::And { .. } | LogicalConstraint::AndSequence { .. } => false,
            LogicalConstraint::Or { or } | LogicalConstraint::Xone { xone: or } => {
                or.iter().any(|bb| evaluate_structural_compatibility(a, bb))
            }
        },

        (Constraint::Logical(a_log), Constraint::Atomic(_)) => match a_log {
            LogicalConstraint::Or { or } | LogicalConstraint::Xone { xone: or } => {
                or.iter().all(|aa| evaluate_structural_compatibility(aa, b))
            }
            LogicalConstraint::And { and }
            | LogicalConstraint::AndSequence { and_sequence: and } => and
                .iter()
                .any(|aa| evaluate_structural_compatibility(aa, b)),
        },
    }
}

// NOTE: a has to be more specific than b
fn subsumed_by(a: &AtomicConstraint, b: &AtomicConstraint) -> bool {
    if a.left_operand != b.left_operand {
        return false;
    }

    let a_val = &a.right_operand;
    let b_val = &b.right_operand;

    match (&a.operator, &b.operator) {
        (Operator::Eq, Operator::Eq) | (Operator::Neq, Operator::Neq) => a_val == b_val,

        (Operator::IsA, Operator::IsA) => is_subclass_of(&a.left_operand, a_val, b_val),

        (Operator::IsPartOf, Operator::IsPartOf)
        | (Operator::IsPartOf, Operator::Eq)
        | (Operator::Eq, Operator::IsPartOf) => is_part_of(&a.left_operand, a_val, b_val),

        (Operator::HasPart, Operator::HasPart)
        | (Operator::HasPart, Operator::Eq)
        | (Operator::Eq, Operator::HasPart) => is_part_of(&a.left_operand, b_val, a_val),

        (Operator::Lt, Operator::Lt)
        | (Operator::Lteq, Operator::Lt)
        | (Operator::Lt, Operator::Lteq)
        | (Operator::Lteq, Operator::Lteq) => lteq(&a.left_operand, a_val, b_val, true),

        (Operator::Gt, Operator::Gt)
        | (Operator::Gteq, Operator::Gt)
        | (Operator::Gt, Operator::Gteq)
        | (Operator::Gteq, Operator::Gteq) => lteq(&a.left_operand, b_val, a_val, true),

        // NOTE: not all implementations provided
        _ => false,
    }
}

fn constraints_are_identical(a: &Option<Vec<Constraint>>, b: &Option<Vec<Constraint>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a_constraints), Some(b_constraints))
            if a_constraints.is_empty() && b_constraints.is_empty() =>
        {
            true
        }
        (Some(a_constraints), Some(b_constraints))
            if a_constraints.len() == b_constraints.len() =>
        {
            b_constraints.iter().all(|bb| {
                a_constraints
                    .iter()
                    .any(|aa| evaluate_structural_identity(aa, bb))
            })
        }
        _ => false,
    }
}

fn evaluate_structural_identity(a: &Constraint, b: &Constraint) -> bool {
    match (a, b) {
        (Constraint::Atomic(a_atom), Constraint::Atomic(b_atom)) => {
            a_atom.left_operand == b_atom.left_operand
                && a_atom.operator == b_atom.operator
                && a_atom.right_operand == b_atom.right_operand
        }

        (Constraint::Logical(a_log), Constraint::Logical(b_log)) => match (a_log, b_log) {
            (LogicalConstraint::And { and: a_list }, LogicalConstraint::And { and: b_list })
            | (
                LogicalConstraint::AndSequence {
                    and_sequence: a_list,
                },
                LogicalConstraint::AndSequence {
                    and_sequence: b_list,
                },
            ) => {
                a_list.len() == b_list.len()
                    && b_list
                        .iter()
                        .all(|bb| a_list.iter().any(|aa| evaluate_structural_identity(aa, bb)))
            }
            (LogicalConstraint::Or { or: a_list }, LogicalConstraint::Or { or: b_list })
            | (
                LogicalConstraint::Xone { xone: a_list },
                LogicalConstraint::Xone { xone: b_list },
            ) => {
                a_list.len() == b_list.len()
                    && b_list
                        .iter()
                        .all(|bb| a_list.iter().any(|aa| evaluate_structural_identity(aa, bb)))
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::model::policy::{
        AtomicConstraint, Constraint, LogicalConstraint, Operator, Policy, RightOperand, Rule,
    };

    #[test]
    fn test_permissions() {
        let now = Utc::now();
        let required = Policy {
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

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: None,
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(
            negotiate_policy(&required, &offered),
            NegotiationOutcome::Forbidden(_)
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(60)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(
            negotiate_policy(&required, &offered),
            NegotiationOutcome::Forbidden(_)
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(20)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(
            matches!(negotiate_policy(&required, &offered), NegotiationOutcome::Success(policy) if policy == offered)
        );

        let required = Policy {
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

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "spatial".into(),
                    operator: Operator::Eq,
                    right_operand: RightOperand::String("DE".into()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(
            matches!(negotiate_policy(&required, &offered), NegotiationOutcome::Success(policy) if policy == offered)
        );
    }

    #[test]
    fn test_prohibitions() {
        let now = Utc::now();
        let required = Policy {
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
                    left_operand: "file_path".into(),
                    operator: Operator::Eq,
                    right_operand: RightOperand::String("/var/secure".into()),
                })]),
            }]),
            obligation: None,
        };

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(10)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(negotiate_policy(&required, &offered),
            NegotiationOutcome::Success(
                Policy {
                permission,
                prohibition,
                ..
            }) if permission == offered.permission && prohibition == required.prohibition,
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Logical(LogicalConstraint::And {
                    and: vec![
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "temporal".into(),
                            operator: Operator::Lteq,
                            right_operand: RightOperand::String(
                                (now + Duration::days(10)).to_rfc3339(),
                            ),
                        }),
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "file_path".into(),
                            operator: Operator::Eq,
                            right_operand: RightOperand::String("/home/data".into()),
                        }),
                    ],
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(negotiate_policy(&required, &offered),
            NegotiationOutcome::Success(
                Policy {
                permission,
                prohibition,
                ..
            }) if permission == offered.permission && prohibition == required.prohibition,
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Logical(LogicalConstraint::Or {
                    or: vec![
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "temporal".into(),
                            operator: Operator::Lteq,
                            right_operand: RightOperand::String(
                                (now + Duration::days(10)).to_rfc3339(),
                            ),
                        }),
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "file_path".into(),
                            operator: Operator::Eq,
                            right_operand: RightOperand::String("/home/data".into()),
                        }),
                    ],
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(
            negotiate_policy(&required, &offered),
            NegotiationOutcome::Forbidden(_)
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Logical(LogicalConstraint::And {
                    and: vec![
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "temporal".into(),
                            operator: Operator::Lteq,
                            right_operand: RightOperand::String(
                                (now + Duration::days(10)).to_rfc3339(),
                            ),
                        }),
                        Constraint::Atomic(AtomicConstraint {
                            left_operand: "file_path".into(),
                            operator: Operator::Eq,
                            right_operand: RightOperand::String("/var/secure".into()),
                        }),
                    ],
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(
            negotiate_policy(&required, &offered),
            NegotiationOutcome::Forbidden(_)
        ));
    }

    #[test]
    fn test_obligations() {
        let now = Utc::now();
        let required = Policy {
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
            obligation: Some(vec![Rule {
                action: "report".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "interval".into(),
                    operator: Operator::Eq,
                    right_operand: RightOperand::String("monthly".into()),
                })]),
            }]),
        };

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(10)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: None,
        };
        assert!(matches!(
            negotiate_policy(&required, &offered),
            NegotiationOutcome::Forbidden(_)
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(10)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: Some(vec![Rule {
                action: "report".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "interval".into(),
                    operator: Operator::Eq,
                    right_operand: RightOperand::String("daily".into()),
                })]),
            }]),
        };
        assert!(matches!(
            negotiate_policy(&required, &offered),
            NegotiationOutcome::Forbidden(_)
        ));

        let offered = Policy {
            profile: None,
            permission: Some(vec![Rule {
                action: "use".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "temporal".into(),
                    operator: Operator::Lteq,
                    right_operand: RightOperand::String((now + Duration::days(10)).to_rfc3339()),
                })]),
            }]),
            prohibition: None,
            obligation: Some(vec![Rule {
                action: "report".into(),
                constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                    left_operand: "interval".into(),
                    operator: Operator::Eq,
                    right_operand: RightOperand::String("monthly".into()),
                })]),
            }]),
        };
        assert!(matches!(negotiate_policy(&required, &offered),
            NegotiationOutcome::Success(
                Policy {
                permission,
                obligation,
                ..
            }) if permission == offered.permission && obligation == required.obligation,
        ));
    }
}
