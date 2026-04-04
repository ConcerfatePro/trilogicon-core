# Trilogicon V1 Private Test Plan

## Target
Release candidate: `v1.0.0-rc1`

## Goal
Run a small private technical test phase focused on correctness, stability, and operator experience.

## What this phase is for
- confirming the README/runbook works for real operators
- confirming two-node setup works repeatedly
- confirming restart behavior is stable
- confirming valid transactions work correctly
- confirming invalid transactions fail cleanly
- finding bugs before any broader public release

## Test areas
- clone + build
- `cargo test`
- two-node shared-genesis setup
- valid transfer flow
- restart and resend flow
- invalid transaction rejection
- chain persistence after restart

## Notes
This phase is for bug discovery, docs cleanup, and reliability hardening.
It is not for adding new V1 features.
