# Trilogicon V2 freeze

## Status

**V2 is frozen** for the **reference `node` implementation** as of the **V2 release tag** (see [`releases/v2.0.0.md`](releases/v2.0.0.md) and git tag `v2.0.0` or the tag your maintainers chose—see checkpoint for naming).

“Frozen” means: the **V2 node-hardening scope** defined in [`v2_scope.md`](v2_scope.md) is **complete** for that tag; further changes on the **V2 line** are limited to the **Allowed** list below. **New protocol or consensus behavior** belongs under **V3** planning ([`v3_scope.md`](v3_scope.md)), not silent edits to V2 docs.

---

## What V2 is (frozen definition)

- **Same V1 protocol validity** for blocks and transactions (see [`v1_scope.md`](v1_scope.md), [`v1_freeze.md`](v1_freeze.md)).
- **Node hardening:** persistence (`chain.blocks` V2 framing + legacy load), restart semantics, genesis binding, pending queue, TCP session/sync bounds, mempool **local** policy, operator messaging.
- **Linear chain only:** no fork choice, no reorg in the V2 reference node.

Full delivery checklist: [`v2_checkpoint.md`](v2_checkpoint.md).

---

## Not changing during the V2 freeze (except via V3)

- Transaction payload, signature, hash, and nonce **consensus** rules.
- Block structure and `basic_validate` **consensus** rules.
- Fee burn rule and genesis allocation model **as consensus-relevant**.
- **Linear-only** committed history in the reference node (extension of a single tip).
- The **meaning** of “valid block” and “valid transaction” for consensus paths.

---

## Allowed on `main` after V2 freeze (V2 maintenance)

- **Bug fixes** that restore documented behavior or close security issues **without** changing the agreed protocol semantics.
- **Tests** and **CI** hygiene.
- **Documentation** clarifications, runbook fixes, [`modules.md`](modules.md) layout updates that **do not** redefine validity.
- **V2.1-style polish** explicitly labeled as non-protocol: e.g. structured logging, optional tooling, dev ergonomics—see [`v2_checkpoint.md`](v2_checkpoint.md) deferrals.
- **Reference `dev-test-ui`** and other **non-protocol** helpers (localhost-only, not part of consensus).

---

## Not allowed without a new protocol version doc (V3+)

- Fork choice, reorg support, or multiple committed tips.
- Changes to signed transaction or block fields that affect validity.
- Fee routing, new issuance, validator/staking **as protocol rules**.
- Chain ID in signed payloads or other identity fields—unless specified in a **V3+ protocol delta**.

---

## If you need to change consensus

1. Update or create a **protocol scope** document (start from [`v3_scope.md`](v3_scope.md)).
2. Update [`modules.md`](modules.md) consensus boundary.
3. Add tests and rejection-matrix coverage **before** or **with** the change.
4. Tag or document a **new protocol / release era**; do not imply V2 still applies to the new behavior without saying so.

This file is the **project discipline** statement for V2 closure. **Future-me:** read this before merging anything that smells like “small consensus tweak.”
