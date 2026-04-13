//! Operator-facing error strings: stable tags for major failure classes (see README + `operator_msg`).

use node::data_dir_bind::verify_or_create_binding;
use node::genesis::{Genesis, GenesisAllocation};
use node::operator_msg::PFX_STARTUP;
use node::types::Address;

fn one_addr_genesis(label: &str) -> Genesis {
    Genesis {
        allocations: vec![GenesisAllocation {
            address: Address::new(label).0,
            balance: 1_000,
        }],
    }
}

#[test]
fn genesis_bind_mismatch_strings_tag_startup_fail_closed() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "trilog_obs_bind_{}_{}",
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let g1 = one_addr_genesis("obs_a");
    let g2 = one_addr_genesis("obs_b");
    verify_or_create_binding(&dir, &g1).unwrap();
    let err = verify_or_create_binding(&dir, &g2).unwrap_err();
    assert!(err.contains(PFX_STARTUP), "{err}");
    assert!(err.contains("fail-closed"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
