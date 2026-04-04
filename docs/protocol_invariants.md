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