//! M5-3: exit aggregation over the twelve diagnostics.

use nsd::config::{Config, PolicyConfig, Severity};
use nsd::policy::exit::exit_status;

const E101: &str = "NSD-E101";
const E102: &str = "NSD-E102";
const V101: &str = "NSD-V101";
const V102: &str = "NSD-V102";
const S101: &str = "NSD-S101";
const S102: &str = "NSD-S102";
const A101: &str = "NSD-A101";
const A102: &str = "NSD-A102";
const G101: &str = "NSD-G101";
const G102: &str = "NSD-G102";
const C101: &str = "NSD-C101";
const C102: &str = "NSD-C102";

const SEVERITIES: [Severity; 3] = [Severity::Deny, Severity::Warn, Severity::Off];

fn default_policy() -> PolicyConfig {
    Config::default().policy
}

fn status(codes: &[&str]) -> u8 {
    exit_status(codes.iter().copied(), &default_policy(), false)
}

#[test]
fn test_no_diagnostics_exit_0() {
    assert_eq!(status(&[]), 0);
}

#[test]
fn test_each_default_deny_code_exits_1() {
    for code in [E101, E102, V101, V102, S101] {
        assert_eq!(status(&[code]), 1, "{code}");
    }
}

#[test]
fn test_each_error_code_exits_2() {
    for code in [A101, A102, G101, G102, C102] {
        assert_eq!(status(&[code]), 2, "{code}");
    }
}

#[test]
fn test_regression_and_error_together_exit_3() {
    for deny in [E101, E102, V101, V102, S101] {
        for error in [A101, A102, G101, G102, C102] {
            assert_eq!(status(&[deny, error]), 3, "{deny} {error}");
            assert_eq!(status(&[error, deny]), 3, "{error} {deny}");
        }
    }
}

#[test]
fn test_c101_and_default_s102_are_exit_neutral() {
    assert_eq!(status(&[C101]), 0);
    assert_eq!(status(&[S102]), 0);
    assert_eq!(status(&[C101, S102]), 0);
}

#[test]
fn test_warn_downgrade_makes_a_code_exit_neutral() {
    let denied = [E101, E102, V101, V102];
    for severity in [Severity::Warn, Severity::Off] {
        for downgraded in denied {
            let mut policy = default_policy();
            match downgraded {
                E101 => policy.nsd_e101 = severity,
                E102 => policy.nsd_e102 = severity,
                V101 => policy.nsd_v101 = severity,
                _ => policy.nsd_v102 = severity,
            }
            assert_eq!(exit_status([downgraded], &policy, false), 0, "{downgraded}");
            for other in denied.into_iter().filter(|code| *code != downgraded) {
                assert_eq!(exit_status([other], &policy, false), 1, "{other}");
            }
            assert_eq!(exit_status([downgraded, A102], &policy, false), 2);
        }
    }
}

#[test]
fn test_s102_deny_exits_1() {
    let policy = PolicyConfig {
        nsd_s102: Severity::Deny,
        ..default_policy()
    };
    assert_eq!(exit_status([S102], &policy, false), 1);
    assert_eq!(exit_status([S102, A101], &policy, false), 3);
    assert_eq!(exit_status([S102], &policy, true), 1);
}

#[test]
fn test_allow_new_suppressions_neutralises_only_s101() {
    let policy = default_policy();
    assert_eq!(exit_status([S101], &policy, true), 0);
    assert_eq!(exit_status([S101, E101], &policy, true), 1);
    assert_eq!(exit_status([S101, V102], &policy, true), 1);
    assert_eq!(exit_status([S101, E102], &policy, true), 1);
    assert_eq!(exit_status([S101, A102], &policy, true), 2);
    assert_eq!(exit_status([S101, E101, A102], &policy, true), 3);
}

#[test]
fn test_repository_policy_cannot_neutralise_s101() {
    for e101 in SEVERITIES {
        for e102 in SEVERITIES {
            for v101 in SEVERITIES {
                for v102 in SEVERITIES {
                    for s102 in SEVERITIES {
                        let policy = PolicyConfig {
                            nsd_e101: e101,
                            nsd_e102: e102,
                            nsd_v101: v101,
                            nsd_v102: v102,
                            nsd_s102: s102,
                        };
                        assert_eq!(exit_status([S101], &policy, false), 1);
                    }
                }
            }
        }
    }
}

#[test]
fn test_unknown_code_fails_closed_as_error() {
    for code in ["NSD-X999", "E101", "", "nsd-e101"] {
        assert_eq!(status(&[code]), 2, "{code:?}");
    }
    assert_eq!(status(&["NSD-X999", E101]), 3);
}

#[test]
fn test_exit_is_order_and_multiplicity_independent() {
    let codes = [E101, A102, S102, C101, S101];
    let expected = status(&codes);
    assert_eq!(expected, 3);
    for rotation in 0..codes.len() {
        let mut rotated = codes.to_vec();
        rotated.rotate_left(rotation);
        assert_eq!(status(&rotated), expected);
        rotated.reverse();
        assert_eq!(status(&rotated), expected);
        let doubled: Vec<&str> = rotated.iter().chain(codes.iter()).copied().collect();
        assert_eq!(status(&doubled), expected);
    }
    assert_eq!(status(&[E101, E101, E101]), 1);
    assert_eq!(status(&[A102, A102]), 2);
}
