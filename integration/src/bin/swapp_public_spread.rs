use integration::helpers::{
    account_component_from_package, build_project_in_dir, compute_p2id_tag_felt,
    compute_p2id_tag_for_local_account, setup_client, AccountCreationConfig, ClientSetup,
};
use integration::swapp_state::SwappTestState;

use anyhow::{Context, Result};
use miden_client::{
    account::component::BasicWallet,
    auth::AuthSecretKey,
    note::{NoteMetadata, NoteType},
    transaction::{OutputNote, TransactionRequestBuilder},
    Felt, Word,
};
use miden_core::FieldElement;
use miden_protocol::{
    account::{AccountBuilder, AccountStorageMode, AccountType},
    asset::{Asset, FungibleAsset},
    note::{NoteAssets, NoteAttachment, NoteDetails},
};
use miden_standards::account::auth::AuthFalcon512Rpo;
use miden_standards::note::utils::build_p2id_recipient;
use miden_swapp::PswapNote;
use rand::RngCore;
use tokio::time::Duration;

/// Public Spread Test:
/// - Alice offers 25 USDT for 20 ETH
/// - Bob offers 20 ETH for 20 USDT
/// - Solver consumes both, earns 5 USDT spread

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== Public Spread Test ===\n");

    // Load persisted state
    let state = SwappTestState::load()?;
    let faucet1_id = state.faucet1_id()?; // USDT
    let faucet2_id = state.faucet2_id()?; // ETH
    let alice_id = state.alice_id()?;
    let bob_id = state.bob_id()?;

    println!("USDT Faucet: {:?}", faucet1_id);
    println!("ETH Faucet: {:?}", faucet2_id);
    println!("Alice: {:?}", alice_id);
    println!("Bob: {:?}\n", bob_id);

    // Setup client
    let ClientSetup {
        mut client,
        keystore,
    } = setup_client().await?;
    client.sync_state().await?;

    // Create Solver account with custom basic-wallet component (same as Bob in setup)
    println!("Creating Solver account with custom basic-wallet...");
    let account_package = std::sync::Arc::new(
        build_project_in_dir(std::path::Path::new("contracts/basic-wallet"), true)
            .context("Failed to build basic-wallet contract")?,
    );

    let solver_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let solver_custom_component =
        account_component_from_package(account_package.clone(), &solver_account_cfg)
            .context("Failed to create Solver's account component")?;

    let mut init_seed = [0u8; 32];
    client.rng().fill_bytes(&mut init_seed);

    let key_pair_solver = AuthSecretKey::new_falcon512_rpo();

    let solver_account = AccountBuilder::new(init_seed)
        .account_type(AccountType::RegularAccountImmutableCode)
        .storage_mode(AccountStorageMode::Public)
        .with_auth_component(AuthFalcon512Rpo::new(
            key_pair_solver.public_key().to_commitment(),
        ))
        .with_component(solver_custom_component)
        .with_component(BasicWallet)
        .build()
        .unwrap();

    client.add_account(&solver_account, false).await?;
    keystore.add_key(&key_pair_solver).unwrap();

    let solver_id = solver_account.id();
    println!("Solver: {:?}\n", solver_id);
    client.sync_state().await?;

    //------------------------------------------------------------
    // Alice creates swap note: 25 USDT for 20 ETH
    //------------------------------------------------------------
    println!("[1] Alice creates swap note (25 USDT -> 20 ETH)");

    let alice_swap_note = PswapNote::create(
        alice_id,
        Asset::Fungible(FungibleAsset::new(faucet1_id, 25)?),
        Asset::Fungible(FungibleAsset::new(faucet2_id, 20)?),
        NoteType::Public,
        NoteAttachment::default(),
        client.rng(),
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Alice's swap note: {:?}", e))?;

    let tx_id = client
        .submit_new_transaction(
            alice_id,
            TransactionRequestBuilder::new()
                .own_output_notes(vec![OutputNote::Full(alice_swap_note.clone())])
                .build()
                .unwrap(),
        )
        .await?;
    println!("Published. TX: {:?}", tx_id);

    //------------------------------------------------------------
    // Bob creates swap note: 20 ETH for 20 USDT
    //------------------------------------------------------------
    println!("\n[2] Bob creates swap note (20 ETH -> 20 USDT)");

    let bob_swap_note = PswapNote::create(
        bob_id,
        Asset::Fungible(FungibleAsset::new(faucet2_id, 20)?),
        Asset::Fungible(FungibleAsset::new(faucet1_id, 20)?),
        NoteType::Public,
        NoteAttachment::default(),
        client.rng(),
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Bob's swap note: {:?}", e))?;

    let tx_id = client
        .submit_new_transaction(
            bob_id,
            TransactionRequestBuilder::new()
                .own_output_notes(vec![OutputNote::Full(bob_swap_note.clone())])
                .build()
                .unwrap(),
        )
        .await?;
    println!("Published. TX: {:?}", tx_id);

    // Wait for notes
    println!("\nWaiting for notes to be available...");
    tokio::time::sleep(Duration::from_secs(10)).await;
    client.sync_state().await?;
    client.sync_state().await?;

    //------------------------------------------------------------
    // Solver consumes both notes
    //------------------------------------------------------------
    println!("\n[3] Solver consuming both swap notes");

    // Note args: [tag, surplus, inflight, input]
    let _solver_p2id_tag_felt = compute_p2id_tag_felt(solver_id);

    let alice_note_args = Word::from([
        Felt::ZERO,
        Felt::ZERO,    // surplus = 5 USDT
        Felt::new(20), // inflight = 20 ETH
        Felt::ZERO,    // input = 0
    ]);

    let bob_note_args = Word::from([
        Felt::ZERO,
        Felt::ZERO,
        Felt::new(20), // inflight = 20 USDT
        Felt::ZERO,
    ]);

    // P2ID for Alice (20 ETH) and Bob (20 USDT) via PswapNote
    let (alice_p2id_note, _) = PswapNote::create_output_notes(&alice_swap_note, solver_id, 0, 20)
        .map_err(|e| anyhow::anyhow!("Alice P2ID: {:?}", e))?;

    let (bob_p2id_note, _) = PswapNote::create_output_notes(&bob_swap_note, solver_id, 0, 20)
        .map_err(|e| anyhow::anyhow!("Bob P2ID: {:?}", e))?;

    // P2ID for Solver (5 USDT spread) - simple P2ID note
    let solver_serial = Word::from([
        alice_swap_note.recipient().serial_num()[0] + Felt::new(2),
        alice_swap_note.recipient().serial_num()[1] + Felt::new(2),
        alice_swap_note.recipient().serial_num()[2] + Felt::new(2),
        alice_swap_note.recipient().serial_num()[3] + Felt::new(2),
    ]);
    let solver_p2id_tag = compute_p2id_tag_for_local_account(solver_id);
    let solver_p2id_recipient = build_p2id_recipient(solver_id, solver_serial)?;
    let solver_p2id_note = miden_protocol::note::Note::new(
        NoteAssets::new(vec![Asset::Fungible(FungibleAsset::new(faucet1_id, 5)?)])
            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
        NoteMetadata::new(solver_id, NoteType::Public, solver_p2id_tag),
        solver_p2id_recipient.clone(),
    );

    println!("Alice P2ID: {:?}", alice_p2id_note.id());
    println!("Bob P2ID: {:?}", bob_p2id_note.id());
    println!("Solver P2ID: {:?}", solver_p2id_note.id());

    // Build expected future notes
    let expected_future_notes = vec![
        (
            NoteDetails::from(&alice_p2id_note),
            alice_p2id_note.metadata().tag(),
        ),
        (
            NoteDetails::from(&bob_p2id_note),
            bob_p2id_note.metadata().tag(),
        ),
    ];

    // Submit consume transaction
    let consume_request = TransactionRequestBuilder::new()
        .input_notes(vec![
            (alice_swap_note.clone(), Some(alice_note_args)),
            (bob_swap_note.clone(), Some(bob_note_args)),
        ])
        .expected_future_notes(expected_future_notes)
        .expected_output_recipients(vec![
            alice_p2id_note.recipient().clone(),
            bob_p2id_note.recipient().clone(),
        ])
        .own_output_notes(vec![OutputNote::Full(solver_p2id_note.clone())])
        .build()
        .context("Failed to build consume transaction")?;

    let tx_id = client
        .submit_new_transaction(solver_id, consume_request)
        .await
        .context("Failed to execute cross-swap transaction")?;
    println!("\nSolver consumed both notes. TX: {:?}", tx_id);

    println!("Waiting for processing...");
    tokio::time::sleep(Duration::from_secs(60)).await;
    client.sync_state().await?;

    //------------------------------------------------------------
    // Each party consumes their P2ID note
    //------------------------------------------------------------
    println!("\n[4] Solver consuming P2ID (5 USDT)");
    match client
        .submit_new_transaction(
            solver_id,
            TransactionRequestBuilder::new()
                .input_notes(vec![(solver_p2id_note.clone(), None)])
                .build()?,
        )
        .await
    {
        Ok(id) => println!("SUCCESS TX: {:?}", id),
        Err(e) => println!("FAILED: {:?}", e),
    }

    tokio::time::sleep(Duration::from_secs(5)).await;
    client.sync_state().await?;

    println!("\n[5] Alice consuming P2ID (20 ETH)");
    match client
        .submit_new_transaction(
            alice_id,
            TransactionRequestBuilder::new()
                .input_notes(vec![(alice_p2id_note.clone(), None)])
                .build()?,
        )
        .await
    {
        Ok(id) => println!("SUCCESS TX: {:?}", id),
        Err(e) => println!("FAILED: {:?}", e),
    }

    tokio::time::sleep(Duration::from_secs(5)).await;
    client.sync_state().await?;

    println!("\n[6] Bob consuming P2ID (20 USDT)");
    match client
        .submit_new_transaction(
            bob_id,
            TransactionRequestBuilder::new()
                .input_notes(vec![(bob_p2id_note.clone(), None)])
                .build()?,
        )
        .await
    {
        Ok(id) => println!("SUCCESS TX: {:?}", id),
        Err(e) => println!("FAILED: {:?}", e),
    }

    println!("\n=== Test Complete ===");
    println!("Alice: 25 USDT -> 20 ETH");
    println!("Bob: 20 ETH -> 20 USDT");
    println!("Solver: 5 USDT spread profit");

    Ok(())
}
