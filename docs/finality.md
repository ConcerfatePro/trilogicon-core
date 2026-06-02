# Finality and Confirmations

Trilogicon does not currently provide production finality.

V1/V2 have one live linear tip and no reorg repair. V3 planning discusses branch selection and reorg execution, but that is not active behavior.

## How to talk about confirmations

A confirmation count is only depth from the current local tip. More depth usually means more local confidence, but it is not a proof against partitions, adversarial producers, or economic attacks.

Do not describe Trilogicon as having Bitcoin-like, Ethereum-like, PoW, PoS, or BFT finality.

## Integrator guidance

- Treat low-confirmation transfers as reversible in principle.
- Use conservative confirmation thresholds for demos.
- For serious value, wait for a security model that this project does not yet have.
