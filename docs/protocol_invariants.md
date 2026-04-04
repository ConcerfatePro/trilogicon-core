# Trilogicon Protocol Invariants

Protocol invariants are rules that must always remain true if the system is functioning correctly.

These are not optional preferences.  
They are fundamental safety properties.

## 1. Balances never go negative

No valid state transition may result in an account balance below zero.

---

## 2. Nonces must prevent replay

A valid transaction must use the correct expected nonce for the sender account.

Transactions with stale nonces must fail.  
Transactions that violate nonce rules must not change state.

---

## 3. Invalid signatures never authorize transfers

A transfer must never be accepted unless it is correctly authorized according to the protocol’s signature rules.

---

## 4. State transitions must be deterministic

Given the same starting state and the same valid transaction/block sequence, all honest nodes must produce the same resulting state.

---

## 5. Invalid transactions must not mutate state

A transaction that fails validation must not partially apply.

State changes should happen only after validation succeeds.

---

## 6. Invalid blocks must be rejected

A block that violates protocol rules must not be accepted into the canonical chain.

---

## 7. Chain history must remain linked correctly

Each non-genesis block must reference the correct previous block hash according to protocol rules.

Broken chain linkage must fail validation.

---

## 8. Transaction application order must be unambiguous

Transactions inside a block must be applied in a well-defined order.

Nodes must not interpret execution ordering differently.

---

## 9. Honest nodes validating the same chain must agree on validity

The validation model must not depend on hidden local assumptions that create disagreement across honest nodes.

---

## 10. Hashing and serialization rules must be stable within a protocol version

Objects that are hashed or signed must have a canonical representation.

Different encodings of the "same" logical object must not cause validation ambiguity.

---

## 11. Genesis rules are fixed for a given network

Nodes on the same network must share the same genesis configuration and initial protocol assumptions.

---

## 12. Protocol rules are more important than local convenience

No node should accept invalid behavior for convenience, speed, or temporary workaround reasons.

If a rule is painful but necessary for correctness, the rule wins.

---

## 13. V1 fees are burned (monetary effect)

For a valid transfer in V1, the sender’s balance decreases by `amount + fee`, the receiver’s balance increases by `amount`, and **no account gains the `fee`**.

Equivalently: the sum of all account balances decreases by `fee` for each such transaction (assuming no other state changes in the same step). This must remain the documented fee semantics for V1 unless the protocol is explicitly revised.

---

## Automated rejection coverage

These `node` tests name the main adversarial classes for V1 (see also finer-grained tests under each module):

| Adversarial input | Primary test name (`node/src/rejection_matrix_tests.rs`) | Notes |
|-------------------|----------------------------------------------------------|--------|
| Bad signature | `v1_rejects_bad_signature` | `Transaction::basic_validate` |
| Wrong nonce (ahead of expected) | `v1_rejects_wrong_nonce_when_expected_lower` | `State::apply_transaction` |
| Nonce reuse (replay same tx) | `v1_rejects_nonce_reuse_same_signed_transaction` | After one successful apply |
| Insufficient balance | `v1_rejects_insufficient_balance` | `State::apply_transaction` |
| Wrong `previous_hash` | `v1_rejects_wrong_previous_hash_on_append` | `Blockchain::append_block` |
| Wrong block height | `v1_rejects_wrong_block_height_on_append` | `Blockchain::append_block` |
| Skipped nonce inside one block | `v1_rejects_second_transaction_in_block_with_skipped_nonce` | Second tx nonce gap after first applies |
| Malformed tx bytes | `v1_rejects_malformed_transaction_encoding` | `decode_transaction` (truncated) |
| Malformed block bytes | `v1_rejects_malformed_block_encoding_truncated`, `v1_rejects_malformed_block_encoding_trailing_garbage` | `decode_block` |
| Timestamp vs parent (`min_block_interval_secs`) | `v1_rejects_block_timestamp_violating_min_interval_after_parent` | `ConsensusParams` on chain |
| Timestamp vs local clock (`max_future_drift_secs`) | `v1_rejects_block_timestamp_too_far_in_future_vs_local_time_on_network_path` | `try_append_network_block` |
| Wall-clock helper | `v1_consensus_local_time_rule_documented` | `validate_block_vs_local_time` directly |

Mempool FIFO with wrong nonce ordering is covered by `blockchain::tests::append_block_from_mempool_rejects_wrong_nonce_order_without_draining_pool`.