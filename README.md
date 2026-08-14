# ZKP2P Solana Programs

Latest-only Solana implementation of the ZKP2P settlement protocol. The parity baseline is the Solidity contracts at
`zkp2p/zkp2p-contracts@fbe141161fe4138421a21e28715e540dafdfee4f`.

Implementation has not begun. The audited behavioral boundary and state machines are defined in
[`docs/PARITY.md`](docs/PARITY.md); that document is the gate for the parity-first implementation.

## Scope

The repository will contain the SVM equivalents of OrchestratorV3, EscrowV2, StakeVault, RateManagerV1, the unified
payment/dispute verifiers, nullifier binding, whitelist/group policy, and stake-backed dispute protection. Predecessor
versions and unused periphery are intentionally excluded.

## License

MIT
