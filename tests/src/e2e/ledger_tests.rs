//! By default, these tests will run in release mode. This can be disabled
//! by setting environment variable `ANOMA_E2E_DEBUG=true`. For debugging,
//! you'll typically also want to set `RUST_BACKTRACE=1`, e.g.:
//!
//! ```ignore,shell
//! ANOMA_E2E_DEBUG=true RUST_BACKTRACE=1 cargo test e2e::ledger_tests -- --test-threads=1 --nocapture
//! ```
//!
//! To keep the temporary files created by a test, use env var
//! `ANOMA_E2E_KEEP_TEMP=true`.

use std::process::Command;

use anoma::types::token;
use anoma_apps::wallet;
use borsh::BorshSerialize;
use color_eyre::eyre::Result;
use setup::constants::*;

use crate::e2e::setup::{self, find_address, single_node_net, sleep, Bin, Who};
use crate::{run, run_as};

/// Test that when we "run-ledger" with all the possible command
/// combinations from fresh state, the node starts-up successfully for both a
/// validator and non-validator user.
#[test]
fn run_ledger() -> Result<()> {
    let test = setup::single_node_net()?;

    let cmd_combinations = vec![vec!["ledger"], vec!["ledger", "run"]];

    // Start the ledger as a validator
    for args in &cmd_combinations {
        let mut ledger =
            run_as!(test, Who::Validator(0), Bin::Node, args, Some(20))?;
        ledger.exp_string("Anoma ledger node started")?;
        ledger.exp_string("This node is a validator")?;
    }

    // Start the ledger as a non-validator
    for args in &cmd_combinations {
        let mut ledger =
            run_as!(test, Who::NonValidator, Bin::Node, args, Some(20))?;
        ledger.exp_string("Anoma ledger node started")?;
        // TODO: I'm (batconjurer) not sure the intention of this test but this
        // may need to be changed
        ledger.exp_string(
            "This node is a validator (NOT in the active validator set)",
        )?;
        // ledger.exp_string("This node is not a validator")?;
    }

    Ok(())
}

/// In this test we:
/// 1. Start up the ledger
/// 2. Kill the tendermint process
/// 3. Check that the node detects this
/// 4. Check that the node shuts down
#[test]
fn test_anoma_shuts_down_if_tendermint_dies() -> Result<()> {
    let test = setup::single_node_net()?;

    // 1. Run the ledger node
    let mut ledger =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20),)?;

    ledger.exp_string("Anoma ledger node started")?;

    // 2. Kill the tendermint node
    sleep(1);
    Command::new("pkill")
        .args(&["tendermint"])
        .spawn()
        .expect("Test failed")
        .wait()
        .expect("Test failed");

    // 3. Check that anoma detects that the tendermint node is dead
    ledger.exp_string("Tendermint node is no longer running.")?;

    // 4. Check that the ledger node shuts down
    ledger.exp_string("Shutting down Anoma node")?;

    Ok(())
}

/// In this test we:
/// 1. Run the ledger node
/// 2. Shut it down
/// 3. Run the ledger again, it should load its previous state
/// 4. Shut it down
/// 5. Reset the ledger's state
/// 6. Run the ledger again, it should start from fresh state
#[test]
fn run_ledger_load_state_and_reset() -> Result<()> {
    let test = single_node_net()?;

    // 1. Run the ledger node
    let mut ledger =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20),)?;

    ledger.exp_string("Anoma ledger node started")?;
    // There should be no previous state
    ledger.exp_string("No state could be found")?;
    // Wait to commit a block
    ledger.exp_regex(r"Committed block hash.*, height: [0-9]+")?;

    // 2. Shut it down
    ledger.send_control('c')?;
    drop(ledger);

    // 3. Run the ledger again, it should load its previous state
    let mut ledger =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20),)?;

    ledger.exp_string("Anoma ledger node started")?;

    // There should be previous state now
    ledger.exp_string("Last state root hash:")?;

    // 4. Shut it down
    ledger.send_control('c')?;
    drop(ledger);

    // 5. Reset the ledger's state
    let _session = run_as!(
        test,
        Who::Validator(0),
        Bin::Node,
        &["ledger", "reset"],
        Some(10),
    )?;

    // 6. Run the ledger again, it should start from fresh state
    let mut session =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20),)?;

    session.exp_string("Anoma ledger node started")?;

    // There should be no previous state
    session.exp_string("No state could be found")?;

    Ok(())
}

/// In this test we:
/// 1. Run the ledger node
/// 2. Submit a token transfer tx
/// 3. Submit a transaction to update an account's validity predicate
/// 4. Submit a custom tx
/// 5. Submit a tx to initialize a new account
/// 6. Query token balance
#[test]
fn ledger_txs_and_queries() -> Result<()> {
    let test = setup::single_node_net()?;

    // 1. Run the ledger node
    let mut ledger =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20),)?;

    ledger.exp_string("Anoma ledger node started")?;
    ledger.exp_string("started node")?;

    let vp_user = wasm_abs_path(VP_USER_WASM);
    let vp_user = vp_user.to_string_lossy();
    let tx_no_op = wasm_abs_path(TX_NO_OP_WASM);
    let tx_no_op = tx_no_op.to_string_lossy();

    let txs_args = vec![
            // 2. Submit a token transfer tx
            vec![
                "transfer",
                "--source",
                BERTHA,
                "--target",
                ALBERT,
                "--token",
                XAN,
                "--amount",
                "10.1",
                "--fee-amount",
                "0",
                "--gas-limit",
                "0",
                "--fee-token",
                XAN,
            ],
            // 3. Submit a transaction to update an account's validity
            // predicate
            vec![
                "update",
                 "--address",
                 BERTHA,
                 "--code-path",
                 &vp_user,
                 "--fee-amount",
                 "0",
                 "--gas-limit",
                 "0",
                 "--fee-token",
                 XAN,
            ],
            // 4. Submit a custom tx
            vec![
                "tx",
                "--signer",
                BERTHA,
                "--code-path",
                &tx_no_op,
                "--data-path",
                "README.md",
                "--fee-amount",
                "0",
                "--gas-limit",
                "0",
                "--fee-token",
                XAN,
            ],
            // 5. Submit a tx to initialize a new account
            vec![
                "init-account", 
                "--source", 
                BERTHA,
                "--public-key", 
                // Value obtained from `anoma::types::key::ed25519::tests::gen_keypair`
                "200000001be519a321e29020fa3cbfbfd01bd5e92db134305609270b71dace25b5a21168",
                "--code-path",
                &vp_user,
                "--alias",
                "test-account",
                "--fee-amount",
                "0",
                "--gas-limit",
                "0",
                "--fee-token",
                XAN,
            ],
        ];
    for tx_args in &txs_args {
        for &dry_run in &[true, false] {
            let tx_args = if dry_run {
                vec![tx_args.clone(), vec!["--dry-run"]].concat()
            } else {
                tx_args.clone()
            };
            let mut client = run!(test, Bin::Client, tx_args, Some(20))?;

            if !dry_run {
                client.exp_string("Transaction accepted")?;
                client.exp_string("Transaction applied")?;
            }
            client.exp_string("Transaction is valid.")?;
            client.assert_success();
        }
    }

    let query_args_and_expected_response = vec![
        // 6. Query token balance
        (
            vec!["balance", "--owner", BERTHA, "--token", XAN],
            // expect a decimal
            r"XAN: \d+(\.\d+)?",
        ),
    ];
    for (query_args, expected) in &query_args_and_expected_response {
        let mut client = run!(test, Bin::Client, query_args, Some(20))?;
        client.exp_regex(expected)?;

        client.assert_success();
    }

    Ok(())
}

/// In this test we:
/// 1. Run the ledger node
/// 2. Submit an invalid transaction (disallowed by state machine)
/// 3. Shut down the ledger
/// 4. Restart the ledger
/// 5. Submit and invalid transactions (malformed)
#[test]
fn invalid_transactions() -> Result<()> {
    let test = setup::single_node_net()?;

    // 1. Run the ledger node
    let mut ledger =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20))?;
    let keypair = wallet::defaults::daewon_keypair();
    let daewon = keypair.to_string();
    ledger.exp_string("Anoma ledger node started")?;
    ledger.exp_string("started node")?;
    // Wait to commit a block
    // ledger.exp_regex(r"Committed block hash.*, height: [0-9]+")?;

    // 2. Submit a an invalid transaction (trying to mint tokens should fail
    // in the token's VP)
    let tx_data_path = test.base_dir.path().join("tx.data");
    let transfer = token::Transfer {
        source: find_address(&test, DAEWON)?,
        target: find_address(&test, ALBERT)?,
        token: find_address(&test, XAN)?,
        amount: token::Amount::whole(1),
    };
    let data = transfer
        .try_to_vec()
        .expect("Encoding unsigned transfer shouldn't fail");
    let tx_wasm_path = wasm_abs_path(TX_MINT_TOKENS_WASM);
    std::fs::write(&tx_data_path, data).unwrap();
    let tx_wasm_path = tx_wasm_path.to_string_lossy();
    let tx_data_path = tx_data_path.to_string_lossy();

    let tx_args = vec![
        "tx",
        "--code-path",
        &tx_wasm_path,
        "--data-path",
        &tx_data_path,
        "--signing-key",
        &daewon,
        "--fee-amount",
        "0",
        "--gas-limit",
        "0",
        "--fee-token",
        XAN,
    ];

    let mut client = run!(test, Bin::Client, tx_args, Some(20))?;

    client.exp_string("Transaction accepted")?;
    client.exp_string("Transaction applied")?;
    client.exp_string("Transaction is invalid")?;
    client.exp_string(r#""code": "1"#)?;

    client.assert_success();
    ledger.exp_string("some VPs rejected apply_tx storage modification")?;

    // Wait to commit a block
    ledger.exp_regex(r"Committed block hash.*, height: [0-9]+")?;

    // 3. Shut it down
    ledger.send_control('c')?;
    drop(ledger);

    // 4. Restart the ledger
    let mut ledger =
        run_as!(test, Who::Validator(0), Bin::Node, &["ledger"], Some(20),)?;

    ledger.exp_string("Anoma ledger node started")?;

    // There should be previous state now
    ledger.exp_string("Last state root hash:")?;

    // 5. Submit an invalid transactions (invalid token address)
    let tx_args = vec![
        "transfer",
        "--source",
        DAEWON,
        "--signing-key",
        &daewon,
        "--target",
        ALBERT,
        "--token",
        BERTHA,
        "--amount",
        "1_000_000.1",
        "--fee-amount",
        "0",
        "--gas-limit",
        "0",
        "--fee-token",
        XAN,
    ];

    let mut client = run!(test, Bin::Client, tx_args, Some(20))?;

    client.exp_string("Transaction accepted")?;
    client.exp_string("Transaction applied")?;

    client.exp_string("Error trying to apply a transaction")?;

    client.exp_string(r#""code": "4"#)?;

    client.assert_success();
    Ok(())
}
