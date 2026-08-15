#!/usr/bin/env python3
"""Enforce the real-SBF instruction matrix and executable core-line coverage."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


EXPECTED_INSTRUCTIONS = {
    "accept_group_curator",
    "accept_protocol_authority",
    "accept_stake_controller",
    "add_funds",
    "cancel_dispute",
    "cancel_intent",
    "claim_stake",
    "clear_stake_owner",
    "configure_currency",
    "configure_escrow",
    "configure_orchestrator",
    "configure_payment_method",
    "configure_address_group",
    "controller_fund_lock",
    "controller_lock_stake",
    "controller_unlock_stake",
    "create_address_group",
    "create_deposit",
    "create_rate_manager",
    "deposit_stake",
    "extend_intent_expiry",
    "fulfill_intent",
    "increase_stake_lock",
    "initialize_claim_balance",
    "initialize_deposit_whitelist",
    "initialize_protocol",
    "initialize_stake_token_vault",
    "manual_release",
    "prepare_dispute",
    "propose_protocol_authority",
    "propose_stake_controller",
    "prune_expired_intent",
    "release_matured_dispute",
    "remove_funds",
    "resize_stake_lock",
    "resolve_stake_lock",
    "select_stake_owner",
    "set_deposit_allowed_group",
    "set_deposit_dispute_protection",
    "set_deposit_rate_manager",
    "set_deposit_whitelist_member",
    "set_dispute_admissions_paused",
    "set_group_member",
    "set_manager_fee",
    "set_manager_min_liquidity",
    "set_manager_rate",
    "set_rate_manager_config",
    "set_required_signatures",
    "set_risk_window",
    "set_self_group_member",
    "set_taker_authorization",
    "set_verifier_payment_method",
    "set_verifier_witness",
    "set_whitelist_enabled",
    "signal_intent",
    "submit_dispute",
    "update_deposit",
    "update_oracle_quote",
    "withdraw_deposit",
    "withdraw_stake",
}


def check_idl(path: Path) -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    actual = {instruction["name"] for instruction in data["instructions"]}
    if actual != EXPECTED_INSTRUCTIONS:
        missing = sorted(EXPECTED_INSTRUCTIONS - actual)
        unexpected = sorted(actual - EXPECTED_INSTRUCTIONS)
        raise SystemExit(
            f"IDL instruction mismatch: missing={missing}, unexpected={unexpected}"
        )


def check_lcov(path: Path) -> None:
    current: str | None = None
    observed = {"state.rs": 0, "math.rs": 0}
    uncovered: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("SF:"):
            current = Path(line.removeprefix("SF:")).name
        elif current in observed and line.startswith("DA:"):
            line_number, count, *_ = line.removeprefix("DA:").split(",")
            observed[current] += 1
            if int(count) == 0:
                uncovered.append(f"{current}:{line_number}")
        elif line == "end_of_record":
            current = None
    missing_files = sorted(name for name, count in observed.items() if count == 0)
    if missing_files or uncovered:
        raise SystemExit(
            f"core line coverage failed: missing_files={missing_files}, uncovered={uncovered}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("lcov", nargs="?")
    parser.add_argument("--idl-only", action="store_true")
    parser.add_argument("--coverage-only", action="store_true")
    args = parser.parse_args()
    if args.idl_only and args.coverage_only:
        parser.error("--idl-only and --coverage-only are mutually exclusive")

    root = Path(__file__).resolve().parents[1]
    idl_path = root / "target/idl/zkp2p_solana.json"
    lcov_path = Path(args.lcov) if args.lcov else root / "target/coverage/core.lcov"
    if not args.coverage_only:
        check_idl(idl_path)
    if not args.idl_only:
        check_lcov(lcov_path)
    if args.idl_only:
        print("parity IDL gate passed: 60/60 instructions")
    elif args.coverage_only:
        print("core coverage gate passed: 100% executable core lines")
    else:
        print("parity coverage gates passed: 60/60 instructions, 100% executable core lines")


if __name__ == "__main__":
    main()
