# Trilogicon Change Policy

## Purpose

This document defines how design and implementation changes should be handled during development.

Trilogicon should remain flexible during construction, but not directionless.

---

## What can change freely early on

The following can change during early development if doing so improves correctness, clarity, or maintainability:

- internal module organization
- helper function structure
- naming details
- temporary storage approach
- temporary networking approach
- unfinished serialization details
- provisional block production details before testnet freeze

---

## What can change carefully

The following may change, but should be changed deliberately and documented clearly:

- transaction field layout
- block field layout
- address formatting choices
- canonical hashing representation
- chain selection details
- persistence format
- network message structure

Changes in this category should not be made casually because they affect compatibility and mental model stability.

---

## What should not change casually

The following are foundational and should remain stable unless there is a serious reason:

- V1 mission
- V1 narrow scope
- account-based ledger model
- nonce-based replay protection
- deterministic validation requirement
- security-first design priority
- protocol invariants

---

## Freeze points

Development should gradually move from flexibility to stability.

### Spec phase
More freedom to refine rules and structures.

### Core implementation phase
Changes are allowed, but should be increasingly documented.

### Pre-testnet phase
Protocol-shaping changes should become rare and justified.

### Public testnet phase
Breaking changes should be treated very seriously and versioned clearly.

---

## Rule for future ideas

If an idea is valuable but not necessary for V1:

- document it
- do not silently push it into current implementation
- revisit it in the appropriate version planning stage

This helps prevent scope creep.

---

## Why this policy exists

Without a change policy, a protocol can become unstable even if the code keeps compiling.

This document exists to ensure that Trilogicon evolves through controlled refinement rather than random drift.
