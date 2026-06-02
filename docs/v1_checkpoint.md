# V1 Checkpoint

V1 is complete for the intended protocol core.

Implemented:

- accounts with balances and nonces;
- signed transfers;
- deterministic transaction and block validation;
- fee burn;
- shared genesis state;
- block persistence and replay;
- basic two-node sync tests.

Not implemented in V1:

- fork choice or reorgs;
- production finality;
- staking/rewards/governance;
- smart contracts;
- richer network defense.

V2 starts from this baseline and hardens the reference node.
