# Security

Trilogicon is not production blockchain infrastructure. Treat it as a reference node and testnet project.

## Reporting issues

Open a private report with enough detail to reproduce the problem. Include:

- affected command or network path;
- expected vs actual behavior;
- logs with secrets removed;
- steps to reproduce from a clean data directory if possible.

## Do not report as security issues

- Missing smart contracts, staking, governance, bridges, or production finality.
- Lack of fork-choice/reorg execution in the live V1/V2 node.
- Testnet faucet limitations unless they expose secrets or allow unintended payouts.

## Local secrets

`wallet.seed` is a private key seed. Do not commit it, paste it into issues, or reuse it outside disposable test networks.
