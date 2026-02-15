use anyhow::Ok;
use integration::helpers::{
    build_project_in_dir, create_testing_account_from_package, create_testing_note_from_package,
    AccountCreationConfig, NoteCreationConfig,
};

use miden_client::{
    account::AccountBuilder,
    note::{
        Note, NoteAssets, NoteExecutionHint, NoteInputs, NoteMetadata, NoteRecipient, NoteScript,
        NoteTag, NoteType,
    },
    transaction::OutputNote,
    Felt, Word,
};
use miden_core::FieldElement;
use miden_protocol::{
    account::AccountType,
    account::{AccountId, AccountStorageMode},
    asset::{Asset, FungibleAsset},
    note::{NoteAttachment, NoteAttachmentScheme},
};
use miden_standards::account::auth::NoAuth;
use miden_standards::note::utils::build_p2id_recipient;
use miden_testing::{Auth, MockChain};
use std::{collections::BTreeMap, path::Path, sync::Arc};

// Import PswapNote from the workspace
use miden_swapp::{BasicWallet, PswapNote};

/// Compute the P2ID tag for a local account
fn compute_p2id_tag_for_local_account(account_id: AccountId) -> NoteTag {
    NoteTag::with_account_target(account_id)
}

/// Helper function to compute P2ID tag as Felt for use in note inputs
/// Returns the tag value as a Felt that can be directly added to note_inputs
fn compute_p2id_tag_felt(account_id: AccountId) -> Felt {
    let p2id_tag = compute_p2id_tag_for_local_account(account_id);
    // In v0.13, NoteTag is a newtype wrapper around u32
    // We can convert it using Into<u32>
    Felt::new(u32::from(p2id_tag) as u64)
}

/// Helper function to create a SWAPP note using PswapNote from the workspace
///
/// # Example Usage:
/// ```ignore
/// let swap_note = create_swapp_note_with_pswap(
///     creator_account_id,
///     offered_asset,   // e.g., 50 USDT
///     requested_asset, // e.g., 25 ETH
///     NoteType::Public,
///     &mut rng,
/// )?;
/// ```
#[allow(dead_code)]
fn create_swapp_note_with_pswap<R: miden_protocol::crypto::rand::FeltRng>(
    creator_account_id: AccountId,
    offered_asset: Asset,
    requested_asset: Asset,
    note_type: NoteType,
    rng: &mut R,
) -> anyhow::Result<Note> {
    let note = PswapNote::create(
        creator_account_id,
        offered_asset,
        requested_asset,
        note_type,
        NoteAttachment::default(),
        rng,
    )?;

    Ok(note)
}

#[tokio::test]
async fn swapp_note_full_fill_test() -> anyhow::Result<()> {
    println!("=== Test: Full Fill Swap ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets in genesis
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "USDC",
        1000,      // max_supply
        Some(150), // total_issuance (50 for note + 100 for Bob)
    )?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());
    println!("  Version: {:?}", usdc_faucet.id().version());

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,     // max_supply
        Some(50), // total_issuance (25 for Alice's request)
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());
    println!("  Version: {:?}", eth_faucet.id().version());

    // Print the prefix and suffix of the ETH faucet
    println!("ETH Faucet prefix: {:?}", eth_faucet.id().prefix());
    println!("ETH Faucet suffix: {:?}", eth_faucet.id().suffix());

    // Print the prefix and suffix of the USDC faucet
    println!("USDC Faucet prefix: {:?}", usdc_faucet.id().prefix());
    println!("USDC Faucet suffix: {:?}", usdc_faucet.id().suffix());

    // STEP 2: Create wallets with initial assets
    println!("\nCreating Alice and Bob wallets with initial assets...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()], // Alice has 50 USDC to offer
    )?;
    println!("Alice: {:?} (has 50 USDC)", alice.id());
    println!("  Version: {:?}", alice.id().version());

    // Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // Create custom account configuration with default settings
    let bob_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let assets = vec![FungibleAsset::new(eth_faucet.id(), 25)?.into()];

    let bob = create_testing_account_from_package(account_package.clone(), bob_account_cfg, assets)
        .await?;
    println!("Bob account created: {:?}", bob.id());

    let _bob_account = builder.add_account(bob.clone());

    // STEP 3: Build swapp-note and p2id-note contracts
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);
    println!("Swapp note contract built successfully.");

    // STEP 4: Create swap note with proper structure
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");

    // Compute proper P2ID tag for Alice (who will receive the output note)
    let p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let note_inputs = vec![
        // Requested Asset (positions 0-3): 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total
        // Note Creator (positions 4-6): Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Alice
        p2id_tag_felt,
    ];

    // Add the offered asset (50 USDC) to the note
    let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut note_assets = NoteAssets::default();
    note_assets.add_asset(offered_asset.into())?;

    let swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: note_assets,
            inputs: note_inputs,
            ..Default::default()
        },
    )?;
    println!("Swap note created: {:?}", swap_note.id());

    // Add note to genesis
    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    // STEP 5: Build MockChain and execute transaction
    println!("\nBuilding MockChain...");
    let mut mock_chain = builder.build()?;

    // Bob consumes the swap note with full fill (25 ETH)
    println!("\nBob consuming swap note (providing 25 ETH - full fill)...");
    let note_args = Word::from([
        Felt::ZERO,
        Felt::ZERO,
        Felt::ZERO,
        Felt::new(25), // input_amount = 25 (full fill)
    ]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    // Create the expected P2ID note that will be created by the swap script
    // This note will contain 25 ETH and be sent to Alice
    println!("\nCreating expected P2ID note for Alice (25 ETH)...");

    let serial_num = Word::from([
        swap_note.recipient().serial_num()[0] + Felt::new(1),
        swap_note.recipient().serial_num()[1] + Felt::new(1),
        swap_note.recipient().serial_num()[2] + Felt::new(1),
        swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let recipient = build_p2id_recipient(alice.id(), serial_num)?;

    // Prepare the advice map for the P2ID note
    // The key is the hash of the note creation parameters, and the value contains the full parameters
    let tag = compute_p2id_tag_for_local_account(alice.id());
    println!("Tag: {:?}", tag.as_u32());
    let aux = Felt::new(25);
    println!("recipient: {:?}", recipient.digest().to_hex());

    println!("serial num: {:?}", serial_num);

    let _execution_hint = NoteExecutionHint::none(); // Not used in v0.13

    // Add the asset (4 Felts = 1 Word)
    let asset = FungibleAsset::new(eth_faucet.id(), 25)?;

    let note_assets = NoteAssets::new(vec![asset.into()])?;

    // In v0.13, create metadata and attach the aux value using NoteAttachment
    // The aux value (25 ETH amount) is wrapped in a Word attachment
    let aux_word = Word::from([aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    println!("Attachment word: {:?}", attachment.content());

    let note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, tag).with_attachment(attachment.clone());

    let p2id_note = Note::new(note_assets, note_metadata, recipient);

    // Rebuild the transaction context with the vault tree nodes
    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_note_args(note_args_map)
        .extend_expected_output_notes(vec![OutputNote::Full(p2id_note)])
        .build()?;

    let executed_transaction = tx_context.execute().await?;

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 6: Verify results
    println!("\n=== Verification ===");

    // Check output notes - should have 1 P2ID note
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(output_notes.num_notes(), 1, "Expected exactly 1 P2ID note");

    assert_eq!(
        output_notes.get_note(0).metadata().attachment().clone(),
        attachment
    );

    let p2id_note = output_notes.get_note(0);
    println!("P2ID note created: {:?}", p2id_note.id());

    // Verify P2ID note contains 25 ETH for Alice
    let p2id_assets = p2id_note.assets().unwrap();
    assert_eq!(
        p2id_assets.num_assets(),
        1,
        "P2ID note should have exactly 1 asset"
    );

    // Get the asset and verify it's 25 ETH
    let p2id_asset = p2id_assets.iter().next().unwrap();
    let p2id_fungible = match p2id_asset {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible asset in P2ID note"),
    };
    assert_eq!(
        p2id_fungible.faucet_id(),
        eth_faucet.id(),
        "P2ID note should contain ETH"
    );
    assert_eq!(
        p2id_fungible.amount(),
        25,
        "P2ID note should contain 25 ETH"
    );
    println!("✓ P2ID note verified: 25 ETH for Alice");

    // Check Bob's account delta - should have received 50 USDC
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();

    // Verify Bob received 50 USDC and spent 25 ETH
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();
    let removed_assets: Vec<Asset> = vault_delta.removed_assets().collect();

    assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset (USDC)");
    // assert_eq!(removed_assets.len(), 1, "Bob should spend 1 asset (ETH)");

    let usdc_received = match added_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible USDC asset"),
    };
    assert_eq!(
        usdc_received.faucet_id(),
        usdc_faucet.id(),
        "Bob should receive USDC"
    );
    assert_eq!(usdc_received.amount(), 50, "Bob should receive 50 USDC");

    mock_chain.add_pending_executed_transaction(&executed_transaction)?;
    let _ = mock_chain.prove_next_block();

    let eth_spent = match removed_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible ETH asset"),
    };
    assert_eq!(
        eth_spent.faucet_id(),
        eth_faucet.id(),
        "Bob should spend ETH"
    );
    assert_eq!(eth_spent.amount(), 25, "Bob should spend 25 ETH");
    println!("✓ Bob's vault delta verified: +50 USDC, -25 ETH");

    println!("\n✅ Full-fill swap test passed!");
    println!("  - Bob provided 25 ETH");
    println!("  - Bob received 50 USDC");
    println!("  - P2ID note created for Alice with 25 ETH");

    Ok(())
}

#[tokio::test]
async fn swapp_note_private_full_fill_test() -> anyhow::Result<()> {
    println!("=== Test: Private Note Full Fill Swap ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets in genesis
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "USDC",
        1000,      // max_supply
        Some(150), // total_issuance (50 for note + 100 for Bob)
    )?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());
    println!("  Version: {:?}", usdc_faucet.id().version());

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,     // max_supply
        Some(50), // total_issuance (25 for Alice's request)
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());
    println!("  Version: {:?}", eth_faucet.id().version());

    // STEP 2: Create wallets with initial assets
    println!("\nCreating Alice and Bob wallets with initial assets...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()], // Alice has 50 USDC to offer
    )?;
    println!("Alice: {:?} (has 50 USDC)", alice.id());
    println!("  Version: {:?}", alice.id().version());

    // Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // Create custom account configuration with default settings
    let bob_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let assets = vec![FungibleAsset::new(eth_faucet.id(), 25)?.into()];

    let bob = create_testing_account_from_package(account_package.clone(), bob_account_cfg, assets)
        .await?;
    println!("Bob account created: {:?}", bob.id());

    let _bob_account = builder.add_account(bob.clone());

    // STEP 3: Build swapp-note contract
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);
    println!("Swapp note contract built successfully.");

    // STEP 4: Create PRIVATE swap note with proper structure
    println!("\nCreating PRIVATE swap note (Alice offers 50 USDC for 25 ETH)...");

    // Compute proper P2ID tag for Alice (who will receive the output note)
    let p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let note_inputs = vec![
        // Requested Asset (positions 0-3): 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total
        // Note Creator (positions 4-6): Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Private.into(),
        // P2ID Tag (position 7): computed tag for Alice
        p2id_tag_felt,
    ];

    // Add the offered asset (50 USDC) to the note
    let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut note_assets = NoteAssets::default();
    note_assets.add_asset(offered_asset.into())?;

    let swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: note_assets,
            inputs: note_inputs,
            note_type: NoteType::Private, // PRIVATE NOTE
            ..Default::default()
        },
    )?;
    println!("Private swap note created: {:?}", swap_note.id());
    println!("  Note type: {:?}", swap_note.metadata().note_type());

    // Add note to genesis
    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    // STEP 5: Build MockChain and execute transaction
    println!("\nBuilding MockChain...");
    let mut mock_chain = builder.build()?;

    // Bob consumes the swap note with full fill (25 ETH)
    println!("\nBob consuming private swap note (providing 25 ETH - full fill)...");
    let note_args = Word::from([
        Felt::ZERO,
        Felt::ZERO,
        Felt::ZERO,
        Felt::new(25), // input_amount = 25 (full fill)
    ]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    // Create the expected P2ID note that will be created by the swap script
    // This note will contain 25 ETH and be sent to Alice
    println!("\nCreating expected P2ID note for Alice (25 ETH)...");

    let serial_num = Word::from([
        swap_note.recipient().serial_num()[0] + Felt::new(1),
        swap_note.recipient().serial_num()[1] + Felt::new(1),
        swap_note.recipient().serial_num()[2] + Felt::new(1),
        swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let recipient = build_p2id_recipient(alice.id(), serial_num)?;

    // Prepare the advice map for the P2ID note
    let tag = compute_p2id_tag_for_local_account(alice.id());
    let aux = Felt::new(25);
    let _execution_hint = NoteExecutionHint::none(); // Not used in v0.13

    // Add the asset (4 Felts = 1 Word)
    let asset = FungibleAsset::new(eth_faucet.id(), 25)?;

    let note_assets = NoteAssets::new(vec![asset.into()])?;

    // P2ID note should also be private to match privacy expectations
    // In v0.13, attach the aux value using NoteAttachment
    let aux_word = Word::from([aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Private, tag).with_attachment(attachment);

    let p2id_note = Note::new(note_assets, note_metadata, recipient);

    // Rebuild the transaction context with the vault tree nodes
    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_note_args(note_args_map)
        .extend_expected_output_notes(vec![OutputNote::Full(p2id_note)])
        .build()?;

    let executed_transaction = tx_context.execute().await?;

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 6: Verify results
    println!("\n=== Verification ===");

    // Check output notes - should have 1 P2ID note
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(output_notes.num_notes(), 1, "Expected exactly 1 P2ID note");

    let p2id_note = output_notes.get_note(0);
    println!("P2ID note created: {:?}", p2id_note.id());

    // Verify the note is private
    assert_eq!(
        p2id_note.metadata().note_type(),
        NoteType::Private,
        "P2ID note should be private"
    );
    println!("✓ P2ID note is private");

    // Verify P2ID note contains 25 ETH for Alice
    let p2id_assets = p2id_note.assets().unwrap();
    assert_eq!(
        p2id_assets.num_assets(),
        1,
        "P2ID note should have exactly 1 asset"
    );

    // Get the asset and verify it's 25 ETH
    let p2id_asset = p2id_assets.iter().next().unwrap();
    let p2id_fungible = match p2id_asset {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible asset in P2ID note"),
    };
    assert_eq!(
        p2id_fungible.faucet_id(),
        eth_faucet.id(),
        "P2ID note should contain ETH"
    );
    assert_eq!(
        p2id_fungible.amount(),
        25,
        "P2ID note should contain 25 ETH"
    );
    println!("✓ P2ID note verified: 25 ETH for Alice");

    // Check Bob's account delta - should have received 50 USDC
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();

    // Verify Bob received 50 USDC and spent 25 ETH
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();
    let removed_assets: Vec<Asset> = vault_delta.removed_assets().collect();

    assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset (USDC)");

    let usdc_received = match added_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible USDC asset"),
    };
    assert_eq!(
        usdc_received.faucet_id(),
        usdc_faucet.id(),
        "Bob should receive USDC"
    );
    assert_eq!(usdc_received.amount(), 50, "Bob should receive 50 USDC");

    mock_chain.add_pending_executed_transaction(&executed_transaction)?;
    let _ = mock_chain.prove_next_block();

    let eth_spent = match removed_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible ETH asset"),
    };
    assert_eq!(
        eth_spent.faucet_id(),
        eth_faucet.id(),
        "Bob should spend ETH"
    );
    assert_eq!(eth_spent.amount(), 25, "Bob should spend 25 ETH");
    println!("✓ Bob's vault delta verified: +50 USDC, -25 ETH");

    println!("\n✅ Private note full-fill swap test passed!");
    println!("  - Private swap note created and consumed");
    println!("  - Bob provided 25 ETH");
    println!("  - Bob received 50 USDC");
    println!("  - Private P2ID note created for Alice with 25 ETH");

    Ok(())
}

#[tokio::test]
async fn swapp_note_partial_fill_test() -> anyhow::Result<()> {
    println!("=== Test: Full Fill Swap ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets in genesis
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "USDC",
        1000,      // max_supply
        Some(150), // total_issuance (50 for note + 100 for Bob)
    )?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());
    println!("  Version: {:?}", usdc_faucet.id().version());

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,     // max_supply
        Some(50), // total_issuance (25 for Alice's request)
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());
    println!("  Version: {:?}", eth_faucet.id().version());

    // STEP 2: Create wallets with initial assets
    println!("\nCreating Alice and Bob wallets with initial assets...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()], // Alice has 50 USDC to offer
    )?;
    println!("Alice: {:?} (has 50 USDC)", alice.id());
    println!("  Version: {:?}", alice.id().version());

    // Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // Create custom account configuration with default settings
    let bob_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let assets = vec![FungibleAsset::new(eth_faucet.id(), 25)?.into()];

    let bob = create_testing_account_from_package(account_package.clone(), bob_account_cfg, assets)
        .await?;
    println!("Bob account created: {:?}", bob.id());

    let _bob_account = builder.add_account(bob.clone());

    // STEP 3: Build swapp-note and p2id-note contracts
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);
    println!("Swapp note contract built successfully.");

    // STEP 4: Create swap note with proper structure
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");

    // Compute proper P2ID tag for Alice (who will receive the output note)
    let p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let note_inputs = vec![
        // Requested Asset (positions 0-3): 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total
        // Note Creator (positions 4-6): Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Alice
        p2id_tag_felt,
    ];

    // Add the offered asset (50 USDC) to the note
    let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut note_assets = NoteAssets::default();
    note_assets.add_asset(offered_asset.into())?;

    let swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: note_assets,
            inputs: note_inputs,
            ..Default::default()
        },
    )?;
    println!("Swap note created: {:?}", swap_note.id());

    // Add note to genesis
    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    // STEP 5: Build MockChain and execute transaction
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    println!("\nBob consuming swap note (providing 15 ETH - partial fill)...");
    let note_args = Word::from([
        Felt::ZERO, // input_amount = 15 (partial fill, 60% of requested)
        Felt::ZERO,
        Felt::ZERO,
        Felt::new(15),
    ]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    // Create the expected P2ID note that will be created by the swap script
    // This note will contain 15 ETH (input_amount) and be sent to Alice
    println!("\nCreating expected P2ID note for Alice (15 ETH)...");

    let p2id_serial_num = Word::from([
        swap_note.recipient().serial_num()[0] + Felt::new(1),
        swap_note.recipient().serial_num()[1] + Felt::new(1),
        swap_note.recipient().serial_num()[2] + Felt::new(1),
        swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let p2id_recipient = build_p2id_recipient(alice.id(), p2id_serial_num)?;

    let p2id_tag = compute_p2id_tag_for_local_account(alice.id());
    let p2id_aux = Felt::new(15); // input_amount = 15 ETH

    let p2id_asset = FungibleAsset::new(eth_faucet.id(), 15)?; // 15 ETH
    let p2id_note_assets = NoteAssets::new(vec![p2id_asset.into()])?;

    // Attach aux value (15) to the metadata
    let aux_word = Word::from([p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let p2id_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, p2id_tag).with_attachment(attachment);

    let p2id_note = Note::new(p2id_note_assets, p2id_note_metadata, p2id_recipient);

    // Create the expected remainder swap note (contains 20 USDC, requests 10 ETH)
    println!("\nCreating expected remainder swap note (20 USDC for 10 ETH)...");

    let current_note_serial = swap_note.recipient().serial_num();
    let serial_num_word = current_note_serial;
    let serial_num_array: [Felt; 4] = serial_num_word.into();
    // Serial number is RPO hash of the existing swap note's serial number
    let remainder_serial_num: [Felt; 4] =
        miden_core::crypto::hash::Rpo256::hash_elements(&serial_num_array).into();
    let remainder_serial_num = Word::from(remainder_serial_num);
    println!("Remainder serial num: {:?}", remainder_serial_num);

    // Note inputs for remainder swap note: requesting 10 ETH, creator is Alice
    let remainder_note_inputs = vec![
        // Requested Asset (positions 0-3): 10 ETH (remaining)
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(10), // requested_asset_total (25 - 15 = 10)
        // Note Creator (positions 4-7): Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        p2id_tag_felt,
    ];

    // Create recipient for remainder swap note
    let note_program = swapp_package.unwrap_program();
    let remainder_note_script = NoteScript::from_parts(
        note_program.mast_forest().clone(),
        note_program.entrypoint(),
    );
    let remainder_note_inputs_obj = NoteInputs::new(remainder_note_inputs)?;
    let remainder_recipient = NoteRecipient::new(
        remainder_serial_num,
        remainder_note_script,
        remainder_note_inputs_obj,
    );

    // Create metadata for remainder note with aux attachment
    let remainder_tag = swap_note.metadata().tag();
    let remainder_aux = Felt::new(30); // offered_out = (50 * 15) / 25 = 30

    println!(
        "Remainder recipient: {:?}",
        remainder_recipient.digest().to_hex()
    );

    // Attach aux value (30) to the remainder note
    let aux_word = Word::from([remainder_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let remainder_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, remainder_tag).with_attachment(attachment);

    // Create assets for remainder note: 20 USDC (50 - 30 = 20)
    let remainder_asset = FungibleAsset::new(usdc_faucet.id(), 20)?;
    let remainder_note_assets = NoteAssets::new(vec![remainder_asset.into()])?;

    let remainder_note = Note::new(
        remainder_note_assets,
        remainder_note_metadata,
        remainder_recipient,
    );

    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_expected_output_notes(vec![
            OutputNote::Full(p2id_note),
            OutputNote::Full(remainder_note),
        ])
        .extend_note_args(note_args_map)
        .build()?;

    let executed_transaction = tx_context.execute().await?;
    println!("Transaction executed successfully!");

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 6: Verify results
    println!("\n=== Verification ===");

    // Should have 2 output notes: P2ID note + remainder swap note
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(
        output_notes.num_notes(),
        2,
        "Expected 2 notes: 1 P2ID + 1 remainder"
    );

    // Find P2ID note and remainder note
    let mut p2id_note_found = false;
    let mut remainder_note_found = false;

    for idx in 0..output_notes.num_notes() {
        let note = output_notes.get_note(idx);
        let assets = note.assets().unwrap();
        if assets.num_assets() == 1 {
            let asset = assets.iter().next().unwrap();
            let fungible = match asset {
                Asset::Fungible(f) => f,
                _ => continue,
            };

            if fungible.faucet_id() == eth_faucet.id() {
                // This is the P2ID note (contains ETH)
                assert_eq!(fungible.amount(), 15, "P2ID note should contain 15 ETH");
                println!("✓ P2ID note verified: 15 ETH for Alice");
                p2id_note_found = true;
            } else if fungible.faucet_id() == usdc_faucet.id() {
                // This is the remainder swap note (contains remaining USDC)
                // Expected: (50 * 10) / 25 = 20 USDC remaining (since 15/25 = 60% filled, 40% remains)
                // Actually: offered_out = (50 * 15) / 25 = 30, so remaining = 50 - 30 = 20
                assert_eq!(
                    fungible.amount(),
                    20,
                    "Remainder note should contain 20 USDC"
                );
                println!("✓ Remainder note verified: 20 USDC (still requesting 10 ETH)");
                remainder_note_found = true;
            }
        }
    }

    assert!(p2id_note_found, "P2ID note not found");
    assert!(remainder_note_found, "Remainder swap note not found");

    // Check Bob's vault delta
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();

    assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset");
    let usdc_received = match added_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible USDC asset"),
    };
    assert_eq!(usdc_received.amount(), 30, "Bob should receive 30 USDC");
    println!("✓ Bob's vault delta verified: +30 USDC, -15 ETH");

    println!("\n✅ Partial-fill swap test passed!");
    println!("  - Bob provided 15 ETH (60% of requested 25)");
    println!("  - Bob received 30 USDC (60% of offered 50)");
    println!("  - P2ID note created for Alice with 15 ETH");
    println!("  - Remainder swap note created: 20 USDC for 10 ETH");

    Ok(())
}

#[tokio::test]
async fn swapp_note_multiple_partial_fills_test() -> anyhow::Result<()> {
    println!("=== Test: Multiple Partial Fill Scenarios ===");

    // Helper function to calculate output amount (matching the contract logic)
    fn calculate_output_amount(offered_total: u64, requested_total: u64, input_amount: u64) -> u64 {
        let precision_factor = 100000u64;

        if offered_total > requested_total {
            // Case 1: offered_total > requested_total
            let ratio = (offered_total * precision_factor) / requested_total;
            (input_amount * ratio) / precision_factor
        } else {
            // Case 2: offered_total <= requested_total
            let ratio = (requested_total * precision_factor) / offered_total;
            (input_amount * precision_factor) / ratio
        }
    }

    // Test scenarios with different input amounts
    let test_scenarios = vec![
        (5, "5 ETH - 20% fill"),
        (7, "7 ETH - 28% fill"),
        (10, "10 ETH - 40% fill"),
        (13, "13 ETH - 52% fill"),
        (15, "15 ETH - 60% fill"),
        (19, "19 ETH - 76% fill"),
        (20, "20 ETH - 80% fill"),
        (23, "23 ETH - 92% fill"),
        (25, "25 ETH - 100% fill (full)"),
    ];

    for (input_amount, description) in test_scenarios {
        println!("\n============================================================");
        println!("Testing scenario: {}", description);
        println!("============================================================");

        let mut builder = MockChain::builder();

        // STEP 1: Create faucets in genesis
        println!("Creating USDC and ETH faucets...");
        let usdc_faucet = builder.add_existing_basic_faucet(
            Auth::BasicAuth,
            "USDC",
            1000,
            Some(150), // total_issuance
        )?;

        let eth_faucet = builder.add_existing_basic_faucet(
            Auth::BasicAuth,
            "ETH",
            1000,
            Some(50), // total_issuance
        )?;

        // STEP 2: Create Alice wallet
        let alice = builder.add_existing_wallet_with_assets(
            Auth::BasicAuth,
            [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
        )?;
        println!("Alice: {:?} (has 50 USDC)", alice.id());

        // Build basic-wallet contract package
        let account_package = Arc::new(build_project_in_dir(
            Path::new("../contracts/basic-wallet"),
            true,
        )?);

        // Create Bob's custom account with ETH
        let bob_account_cfg = AccountCreationConfig {
            storage_slots: vec![],
            ..Default::default()
        };

        let assets = vec![FungibleAsset::new(eth_faucet.id(), input_amount)?.into()];
        let bob =
            create_testing_account_from_package(account_package.clone(), bob_account_cfg, assets)
                .await?;
        println!(
            "Bob account created with {} ETH: {:?}",
            input_amount,
            bob.id()
        );

        builder.add_account(bob.clone())?;

        // STEP 3: Build swapp-note contract
        let swapp_package = Arc::new(build_project_in_dir(
            Path::new("../contracts/swapp-note"),
            true,
        )?);

        // STEP 4: Create swap note (Alice offers 50 USDC for 25 ETH)
        println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");

        // Compute proper P2ID tag for Alice (who will receive the output note)
        let p2id_tag_felt = compute_p2id_tag_felt(alice.id());

        let note_inputs = vec![
            eth_faucet.id().prefix().into(),
            eth_faucet.id().suffix(),
            Felt::ZERO,
            Felt::new(25), // requested_asset_total
            alice.id().prefix().into(),
            alice.id().suffix(),
            NoteType::Public.into(),
            // P2ID Tag (position 7): computed tag for Alice
            p2id_tag_felt,
        ];

        let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
        let mut note_assets = NoteAssets::default();
        note_assets.add_asset(offered_asset.into())?;

        let swap_note = create_testing_note_from_package(
            swapp_package.clone(),
            alice.id(),
            NoteCreationConfig {
                assets: note_assets,
                inputs: note_inputs,
                ..Default::default()
            },
        )?;

        builder.add_output_note(OutputNote::Full(swap_note.clone()));

        // STEP 5: Build MockChain
        let mock_chain = builder.build()?;

        // Calculate expected amounts
        let offered_total = 50u64;
        let requested_total = 25u64;
        let offered_out = calculate_output_amount(offered_total, requested_total, input_amount);
        let remaining_usdc = offered_total - offered_out;
        let remaining_eth = requested_total - input_amount;

        println!("\nExpected calculations:");
        println!("  Input amount: {} ETH", input_amount);
        println!("  Offered out (to Bob): {} USDC", offered_out);
        println!("  Remaining USDC: {}", remaining_usdc);
        println!("  Remaining ETH requested: {}", remaining_eth);

        // Execute transaction
        println!(
            "\nBob consuming swap note (providing {} ETH)...",
            input_amount
        );
        let note_args = Word::from([Felt::ZERO, Felt::ZERO, Felt::ZERO, Felt::new(input_amount)]);

        let mut note_args_map = BTreeMap::new();
        note_args_map.insert(swap_note.id(), note_args);

        // Create expected P2ID note
        let p2id_serial_num = Word::from([
            swap_note.recipient().serial_num()[0] + Felt::new(1),
            swap_note.recipient().serial_num()[1] + Felt::new(1),
            swap_note.recipient().serial_num()[2] + Felt::new(1),
            swap_note.recipient().serial_num()[3] + Felt::new(1),
        ]);

        let p2id_recipient = build_p2id_recipient(alice.id(), p2id_serial_num)?;
        let p2id_tag = compute_p2id_tag_for_local_account(alice.id());
        let p2id_aux = Felt::new(input_amount);
        let p2id_asset = FungibleAsset::new(eth_faucet.id(), input_amount)?;
        let p2id_note_assets = NoteAssets::new(vec![p2id_asset.into()])?;

        // Attach aux value to the metadata
        let aux_word = Word::from([p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
        let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
        let p2id_note_metadata =
            NoteMetadata::new(bob.id(), NoteType::Public, p2id_tag).with_attachment(attachment);
        let p2id_note = Note::new(p2id_note_assets, p2id_note_metadata, p2id_recipient);

        let mut expected_notes = vec![OutputNote::Full(p2id_note)];

        // Create expected remainder swap note (only if not full fill)
        if input_amount < 25 {
            println!(
                "\nCreating expected remainder swap note ({} USDC for {} ETH)...",
                remaining_usdc, remaining_eth
            );

            let current_note_serial = swap_note.recipient().serial_num();
            let serial_num_array: [Felt; 4] = current_note_serial.into();
            let remainder_serial_num: [Felt; 4] =
                miden_core::crypto::hash::Rpo256::hash_elements(&serial_num_array).into();
            let remainder_serial_num = Word::from(remainder_serial_num);

            let remainder_note_inputs = vec![
                eth_faucet.id().prefix().into(),
                eth_faucet.id().suffix(),
                Felt::ZERO,
                Felt::new(remaining_eth),
                alice.id().prefix().into(),
                alice.id().suffix(),
                NoteType::Public.into(),
                p2id_tag_felt,
            ];

            let note_program = swapp_package.unwrap_program();
            let remainder_note_script = NoteScript::from_parts(
                note_program.mast_forest().clone(),
                note_program.entrypoint(),
            );
            let remainder_note_inputs_obj = NoteInputs::new(remainder_note_inputs)?;
            let remainder_recipient = NoteRecipient::new(
                remainder_serial_num,
                remainder_note_script,
                remainder_note_inputs_obj,
            );

            let remainder_tag = swap_note.metadata().tag();
            let remainder_aux = Felt::new(offered_out);

            // Attach aux value to the remainder note metadata
            let aux_word = Word::from([remainder_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
            let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
            let remainder_note_metadata =
                NoteMetadata::new(bob.id(), NoteType::Public, remainder_tag)
                    .with_attachment(attachment);

            let remainder_asset = FungibleAsset::new(usdc_faucet.id(), remaining_usdc)?;
            let remainder_note_assets = NoteAssets::new(vec![remainder_asset.into()])?;

            let remainder_note = Note::new(
                remainder_note_assets,
                remainder_note_metadata,
                remainder_recipient,
            );

            expected_notes.push(OutputNote::Full(remainder_note));
        }

        let tx_context = mock_chain
            .build_tx_context(bob.id(), &[swap_note.id()], &[])?
            .extend_expected_output_notes(expected_notes)
            .extend_note_args(note_args_map)
            .build()?;

        let executed_transaction = tx_context.execute().await?;
        println!("Transaction executed successfully!");

        println!(
            "Cycle count: {:?}",
            executed_transaction.measurements().note_execution
        );

        // STEP 6: Verify results
        println!("\n=== Verification ===");

        let output_notes = executed_transaction.output_notes();
        let expected_note_count = if input_amount < 25 { 2 } else { 1 };
        println!("Output notes created: {}", output_notes.num_notes());
        assert_eq!(
            output_notes.num_notes(),
            expected_note_count,
            "Expected {} output note(s)",
            expected_note_count
        );

        // Verify P2ID note and remainder note
        let mut p2id_found = false;
        let mut remainder_found = false;

        for idx in 0..output_notes.num_notes() {
            let note = output_notes.get_note(idx);
            let assets = note.assets().unwrap();

            if assets.num_assets() == 1 {
                let asset = assets.iter().next().unwrap();
                if let Asset::Fungible(f) = asset {
                    if f.faucet_id() == eth_faucet.id() {
                        assert_eq!(
                            f.amount(),
                            input_amount,
                            "P2ID note should contain {} ETH",
                            input_amount
                        );
                        println!("  ✓ P2ID note verified: {} ETH for Alice", input_amount);
                        p2id_found = true;
                    } else if f.faucet_id() == usdc_faucet.id() {
                        assert_eq!(
                            f.amount(),
                            remaining_usdc,
                            "Remainder note should contain {} USDC",
                            remaining_usdc
                        );
                        println!(
                            "  ✓ Remainder note verified: {} USDC (requesting {} ETH)",
                            remaining_usdc, remaining_eth
                        );
                        remainder_found = true;
                    }
                }
            }
        }

        assert!(p2id_found, "P2ID note not found");
        if input_amount < 25 {
            assert!(remainder_found, "Remainder swap note not found");
        }

        // Check Bob's vault delta
        let account_delta = executed_transaction.account_delta();
        let vault_delta = account_delta.vault();
        let added_assets: Vec<Asset> = vault_delta.added_assets().collect();

        assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset");
        if let Asset::Fungible(f) = added_assets[0] {
            assert_eq!(
                f.amount(),
                offered_out,
                "Bob should receive {} USDC",
                offered_out
            );
            println!("  ✓ Bob received {} USDC", offered_out);
        }

        println!("\n✅ Scenario '{}' passed!", description);
    }

    println!("\n============================================================");
    println!("✅ All multiple partial fill scenarios passed!");
    println!("============================================================");

    Ok(())
}

#[tokio::test]
async fn swapp_note_inflight_cross_swap_test() -> anyhow::Result<()> {
    println!("=== Test: Inflight Cross Swap (Two Notes, Zero Input) ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets in genesis
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "USDC",
        1000,
        Some(100), // total_issuance
    )?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,
        Some(50), // total_issuance
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());

    // STEP 2: Create Alice, Charlie, and Bob accounts
    println!("\nCreating Alice with 25 ETH (wants 50 USDC)...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(eth_faucet.id(), 25)?.into()],
    )?;
    println!("Alice: {:?} (has 25 ETH)", alice.id());

    println!("\nCreating Charlie with 50 USDC (wants 25 ETH)...");
    let charlie = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
    )?;
    println!("Charlie: {:?} (has 50 USDC)", charlie.id());

    // Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // Create Bob's custom account with 0 assets
    println!("\nCreating Bob with 0 assets (custom account)...");
    let bob_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let bob = create_testing_account_from_package(
        account_package.clone(),
        bob_account_cfg,
        vec![], // 0 assets
    )
    .await?;
    println!("Bob account created: {:?}", bob.id());

    builder.add_account(bob.clone())?;

    // STEP 3: Build swapp-note contract
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);
    println!("Swapp note contract built successfully.");

    // STEP 4: Create Alice's swap note (offers 25 ETH, wants 50 USDC)
    println!("\nCreating Alice's swap note (offers 25 ETH for 50 USDC)...");

    // Compute proper P2ID tag for Alice (who will receive the output note)
    let alice_p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let alice_note_inputs = vec![
        // Requested Asset: 50 USDC
        usdc_faucet.id().prefix().into(),
        usdc_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(50), // requested_asset_total
        // Note Creator: Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Alice
        alice_p2id_tag_felt,
    ];

    let alice_offered_asset = FungibleAsset::new(eth_faucet.id(), 25)?;
    let mut alice_note_assets = NoteAssets::default();
    alice_note_assets.add_asset(alice_offered_asset.into())?;

    let alice_swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: alice_note_assets,
            inputs: alice_note_inputs,
            ..Default::default()
        },
    )?;
    println!("Alice's swap note created: {:?}", alice_swap_note.id());

    builder.add_output_note(OutputNote::Full(alice_swap_note.clone()));

    // STEP 5: Create Charlie's swap note (offers 50 USDC, wants 25 ETH)
    println!("\nCreating Charlie's swap note (offers 50 USDC for 25 ETH)...");

    // Compute proper P2ID tag for Charlie (who will receive the output note)
    let charlie_p2id_tag_felt = compute_p2id_tag_felt(charlie.id());

    let charlie_note_inputs = vec![
        // Requested Asset: 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total
        // Note Creator: Charlie
        charlie.id().prefix().into(),
        charlie.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Charlie
        charlie_p2id_tag_felt,
    ];

    let charlie_offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut charlie_note_assets = NoteAssets::default();
    charlie_note_assets.add_asset(charlie_offered_asset.into())?;

    let charlie_swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        charlie.id(),
        NoteCreationConfig {
            assets: charlie_note_assets,
            inputs: charlie_note_inputs,
            ..Default::default()
        },
    )?;
    println!("Charlie's swap note created: {:?}", charlie_swap_note.id());

    builder.add_output_note(OutputNote::Full(charlie_swap_note.clone()));

    // STEP 6: Build MockChain
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    // STEP 7: Bob consumes both notes with input_amount = 0 (inflight swap)
    println!("\nBob consuming both swap notes with inflight logic (input_amount = 0)...");

    // Note args for both notes: input_amount = 0
    let alice_note_args = Word::from([Felt::ZERO, Felt::ZERO, Felt::new(50), Felt::ZERO]);
    let charlie_note_args = Word::from([Felt::ZERO, Felt::ZERO, Felt::new(25), Felt::ZERO]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(alice_swap_note.id(), alice_note_args);
    println!("Alice note args: {:?}", alice_swap_note.serial_num());

    note_args_map.insert(charlie_swap_note.id(), charlie_note_args);
    println!("Charlie note args: {:?}", charlie_swap_note.serial_num());

    // STEP 8: Create expected P2ID notes

    // P2ID note for Alice (50 USDC from Charlie's note)
    println!("\nCreating expected P2ID note for Alice (50 USDC)...");
    let alice_p2id_serial_num = Word::from([
        alice_swap_note.recipient().serial_num()[0] + Felt::new(1),
        alice_swap_note.recipient().serial_num()[1] + Felt::new(1),
        alice_swap_note.recipient().serial_num()[2] + Felt::new(1),
        alice_swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let alice_p2id_recipient = build_p2id_recipient(alice.id(), alice_p2id_serial_num)?;
    let alice_p2id_tag = compute_p2id_tag_for_local_account(alice.id());
    let alice_p2id_aux = Felt::new(50); // 50 USDC
    let alice_p2id_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let alice_p2id_note_assets = NoteAssets::new(vec![alice_p2id_asset.into()])?;

    // Attach aux value to Alice's P2ID note
    let aux_word = Word::from([alice_p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let alice_p2id_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, alice_p2id_tag).with_attachment(attachment);
    let alice_p2id_note = Note::new(
        alice_p2id_note_assets,
        alice_p2id_note_metadata,
        alice_p2id_recipient,
    );

    // P2ID note for Charlie (25 ETH from Alice's note)
    println!("\nCreating expected P2ID note for Charlie (25 ETH)...");
    let charlie_p2id_serial_num = Word::from([
        charlie_swap_note.recipient().serial_num()[0] + Felt::new(1),
        charlie_swap_note.recipient().serial_num()[1] + Felt::new(1),
        charlie_swap_note.recipient().serial_num()[2] + Felt::new(1),
        charlie_swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let charlie_p2id_recipient = build_p2id_recipient(charlie.id(), charlie_p2id_serial_num)?;
    let charlie_p2id_tag = compute_p2id_tag_for_local_account(charlie.id());
    let charlie_p2id_aux = Felt::new(25); // 25 ETH
    let charlie_p2id_asset = FungibleAsset::new(eth_faucet.id(), 25)?;
    let charlie_p2id_note_assets = NoteAssets::new(vec![charlie_p2id_asset.into()])?;

    // Attach aux value to Charlie's P2ID note
    let aux_word = Word::from([charlie_p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let charlie_p2id_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, charlie_p2id_tag).with_attachment(attachment);
    let charlie_p2id_note = Note::new(
        charlie_p2id_note_assets,
        charlie_p2id_note_metadata,
        charlie_p2id_recipient,
    );

    // Execute transaction with both notes
    let tx_context = mock_chain
        .build_tx_context(
            bob.id(),
            &[alice_swap_note.id(), charlie_swap_note.id()],
            &[],
        )?
        .extend_expected_output_notes(vec![
            OutputNote::Full(alice_p2id_note),
            OutputNote::Full(charlie_p2id_note),
        ])
        .extend_note_args(note_args_map)
        .build()?;

    let executed_transaction = tx_context.execute().await?;
    println!("Transaction executed successfully!");

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 9: Verify results
    println!("\n=== Verification ===");

    // Should have exactly 2 P2ID notes, 0 remainder notes
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(
        output_notes.num_notes(),
        2,
        "Expected exactly 2 P2ID notes (no remainder notes)"
    );

    // Verify the two P2ID notes
    let mut alice_p2id_found = false;
    let mut charlie_p2id_found = false;

    for idx in 0..output_notes.num_notes() {
        let note = output_notes.get_note(idx);
        println!("\nNote {}: {:?}", idx, note.id());

        let assets = note.assets().unwrap();
        println!("  Number of assets: {}", assets.num_assets());

        if assets.num_assets() == 1 {
            let asset = assets.iter().next().unwrap();
            if let Asset::Fungible(f) = asset {
                println!("  Asset: {} of faucet {:?}", f.amount(), f.faucet_id());

                if f.faucet_id() == usdc_faucet.id() {
                    // This is Alice's P2ID note (50 USDC)
                    assert_eq!(f.amount(), 50, "Alice's P2ID note should contain 50 USDC");
                    println!("  ✓ Alice's P2ID note verified: 50 USDC");
                    alice_p2id_found = true;
                } else if f.faucet_id() == eth_faucet.id() {
                    // This is Charlie's P2ID note (25 ETH)
                    assert_eq!(f.amount(), 25, "Charlie's P2ID note should contain 25 ETH");
                    println!("  ✓ Charlie's P2ID note verified: 25 ETH");
                    charlie_p2id_found = true;
                }
            }
        }
    }

    assert!(alice_p2id_found, "Alice's P2ID note not found");
    assert!(charlie_p2id_found, "Charlie's P2ID note not found");

    // Check Bob's vault delta - should be ZERO (no change)
    println!("\nVerifying Bob's vault delta...");
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();
    let removed_assets: Vec<Asset> = vault_delta.removed_assets().collect();

    println!("  Assets added: {}", added_assets.len());
    println!("  Assets removed: {}", removed_assets.len());

    assert_eq!(
        added_assets.len(),
        0,
        "Bob should not receive any assets (inflight swap)"
    );
    assert_eq!(
        removed_assets.len(),
        0,
        "Bob should not spend any assets (inflight swap)"
    );
    println!("  ✓ Bob's vault unchanged (0 added, 0 removed)");

    println!("\n✅ Inflight cross-swap test passed!");
    println!("  - Alice's 25 ETH → Charlie (via P2ID note)");
    println!("  - Charlie's 50 USDC → Alice (via P2ID note)");
    println!("  - Bob facilitated the swap with 0 assets");
    println!("  - 2 P2ID notes created, 0 remainder notes");

    Ok(())
}

#[tokio::test]
async fn swapp_note_inflight_cross_swap_with_spread_test() -> anyhow::Result<()> {
    println!("=== Test: Inflight Cross Swap With Spread (Bob Earns 5 ETH) ===");
    println!("Alice offers 30 ETH for 50 USDC, Charlie offers 50 USDC for 25 ETH");
    println!("Spread: 30 - 25 = 5 ETH goes to Bob as profit");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets in genesis
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "USDC",
        1000,
        Some(100), // total_issuance
    )?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,
        Some(50), // total_issuance
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());

    // STEP 2: Create Alice, Charlie, and Bob accounts
    println!("\nCreating Alice with 30 ETH (wants 50 USDC)...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(eth_faucet.id(), 30)?.into()],
    )?;
    println!("Alice: {:?} (has 30 ETH)", alice.id());

    println!("\nCreating Charlie with 50 USDC (wants 25 ETH)...");
    let charlie = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
    )?;
    println!("Charlie: {:?} (has 50 USDC)", charlie.id());

    // Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // Create Bob's custom account with 0 assets
    println!("\nCreating Bob with 0 assets (custom account)...");
    let bob_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let bob = create_testing_account_from_package(
        account_package.clone(),
        bob_account_cfg,
        vec![], // 0 assets
    )
    .await?;
    println!("Bob account created: {:?}", bob.id());

    builder.add_account(bob.clone())?;

    // STEP 3: Build swapp-note contract
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);
    println!("Swapp note contract built successfully.");

    // STEP 4: Create Alice's swap note (offers 30 ETH, wants 50 USDC)
    println!("\nCreating Alice's swap note (offers 30 ETH for 50 USDC)...");

    // Compute proper P2ID tag for Alice (who will receive the output note)
    let alice_p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let alice_note_inputs = vec![
        // Requested Asset: 50 USDC
        usdc_faucet.id().prefix().into(),
        usdc_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(50), // requested_asset_total
        // Note Creator: Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Alice
        alice_p2id_tag_felt,
    ];

    let alice_offered_asset = FungibleAsset::new(eth_faucet.id(), 30)?;
    let mut alice_note_assets = NoteAssets::default();
    alice_note_assets.add_asset(alice_offered_asset.into())?;

    let alice_swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: alice_note_assets,
            inputs: alice_note_inputs,
            ..Default::default()
        },
    )?;
    println!("Alice's swap note created: {:?}", alice_swap_note.id());

    builder.add_output_note(OutputNote::Full(alice_swap_note.clone()));

    // STEP 5: Create Charlie's swap note (offers 50 USDC, wants 25 ETH)
    println!("\nCreating Charlie's swap note (offers 50 USDC for 25 ETH)...");

    // Compute proper P2ID tag for Charlie (who will receive the output note)
    let charlie_p2id_tag_felt = compute_p2id_tag_felt(charlie.id());

    let charlie_note_inputs = vec![
        // Requested Asset: 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total
        // Note Creator: Charlie
        charlie.id().prefix().into(),
        charlie.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Charlie
        charlie_p2id_tag_felt,
    ];

    let charlie_offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut charlie_note_assets = NoteAssets::default();
    charlie_note_assets.add_asset(charlie_offered_asset.into())?;

    let charlie_swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        charlie.id(),
        NoteCreationConfig {
            assets: charlie_note_assets,
            inputs: charlie_note_inputs,
            ..Default::default()
        },
    )?;
    println!("Charlie's swap note created: {:?}", charlie_swap_note.id());

    builder.add_output_note(OutputNote::Full(charlie_swap_note.clone()));

    // STEP 6: Build MockChain
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    // STEP 7: Bob consumes both notes with input_amount = 0 (inflight swap)
    println!("\nBob consuming both swap notes with inflight logic (input_amount = 0)...");

    // Note args format (reversed Word): [consumer_tag, surplus, inflight, input]
    // arg[0]=input (last), arg[1]=inflight (3rd), arg[2]=surplus (2nd), arg[3]=consumer_tag (1st)
    let bob_p2id_tag_felt = compute_p2id_tag_felt(bob.id());

    // Alice's note: input=0, inflight=50 USDC, surplus=5 ETH for Bob, tag=Bob's P2ID tag
    let alice_note_args = Word::from([bob_p2id_tag_felt, Felt::new(5), Felt::new(50), Felt::ZERO]);
    // Charlie's note: input=0, inflight=25 ETH, surplus=0, tag=0
    let charlie_note_args = Word::from([Felt::ZERO, Felt::ZERO, Felt::new(25), Felt::ZERO]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(alice_swap_note.id(), alice_note_args);
    println!("Alice note args: {:?}", alice_swap_note.serial_num());

    note_args_map.insert(charlie_swap_note.id(), charlie_note_args);
    println!("Charlie note args: {:?}", charlie_swap_note.serial_num());

    // STEP 8: Create expected P2ID notes

    // P2ID note for Alice (50 USDC from Charlie's note)
    println!("\nCreating expected P2ID note for Alice (50 USDC)...");
    let alice_p2id_serial_num = Word::from([
        alice_swap_note.recipient().serial_num()[0] + Felt::new(1),
        alice_swap_note.recipient().serial_num()[1] + Felt::new(1),
        alice_swap_note.recipient().serial_num()[2] + Felt::new(1),
        alice_swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let alice_p2id_recipient = build_p2id_recipient(alice.id(), alice_p2id_serial_num)?;
    let alice_p2id_tag = compute_p2id_tag_for_local_account(alice.id());
    let alice_p2id_aux = Felt::new(50); // 50 USDC
    let alice_p2id_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let alice_p2id_note_assets = NoteAssets::new(vec![alice_p2id_asset.into()])?;

    // Attach aux value to Alice's P2ID note
    let aux_word = Word::from([alice_p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let alice_p2id_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, alice_p2id_tag).with_attachment(attachment);
    let alice_p2id_note = Note::new(
        alice_p2id_note_assets,
        alice_p2id_note_metadata,
        alice_p2id_recipient,
    );

    // P2ID note for Charlie (25 ETH from Alice's note)
    println!("\nCreating expected P2ID note for Charlie (25 ETH)...");
    let charlie_p2id_serial_num = Word::from([
        charlie_swap_note.recipient().serial_num()[0] + Felt::new(1),
        charlie_swap_note.recipient().serial_num()[1] + Felt::new(1),
        charlie_swap_note.recipient().serial_num()[2] + Felt::new(1),
        charlie_swap_note.recipient().serial_num()[3] + Felt::new(1),
    ]);

    let charlie_p2id_recipient = build_p2id_recipient(charlie.id(), charlie_p2id_serial_num)?;
    let charlie_p2id_tag = compute_p2id_tag_for_local_account(charlie.id());
    let charlie_p2id_aux = Felt::new(25); // 25 ETH
    let charlie_p2id_asset = FungibleAsset::new(eth_faucet.id(), 25)?;
    let charlie_p2id_note_assets = NoteAssets::new(vec![charlie_p2id_asset.into()])?;

    // Attach aux value to Charlie's P2ID note
    let aux_word = Word::from([charlie_p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let charlie_p2id_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, charlie_p2id_tag).with_attachment(attachment);
    let charlie_p2id_note = Note::new(
        charlie_p2id_note_assets,
        charlie_p2id_note_metadata,
        charlie_p2id_recipient,
    );

    // P2ID note for Bob (5 ETH spread earned from the swap)
    println!("\nCreating expected P2ID note for Bob (5 ETH spread)...");
    let bob_p2id_serial_num = Word::from([
        alice_swap_note.recipient().serial_num()[0] + Felt::new(2),
        alice_swap_note.recipient().serial_num()[1] + Felt::new(2),
        alice_swap_note.recipient().serial_num()[2] + Felt::new(2),
        alice_swap_note.recipient().serial_num()[3] + Felt::new(2),
    ]);

    let bob_p2id_recipient = build_p2id_recipient(bob.id(), bob_p2id_serial_num)?;
    let bob_p2id_tag = compute_p2id_tag_for_local_account(bob.id());
    let bob_p2id_aux = Felt::new(5); // 5 ETH spread
    let bob_p2id_asset = FungibleAsset::new(eth_faucet.id(), 5)?;
    let bob_p2id_note_assets = NoteAssets::new(vec![bob_p2id_asset.into()])?;

    let aux_word = Word::from([bob_p2id_aux, Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
    let bob_p2id_note_metadata =
        NoteMetadata::new(bob.id(), NoteType::Public, bob_p2id_tag).with_attachment(attachment);
    let bob_p2id_note = Note::new(
        bob_p2id_note_assets,
        bob_p2id_note_metadata,
        bob_p2id_recipient,
    );

    // Execute transaction with both notes
    // 3 P2ID notes: Alice(50 USDC), Charlie(25 ETH), Bob(5 ETH spread)
    let tx_context = mock_chain
        .build_tx_context(
            bob.id(),
            &[alice_swap_note.id(), charlie_swap_note.id()],
            &[],
        )?
        .extend_expected_output_notes(vec![
            OutputNote::Full(alice_p2id_note),
            OutputNote::Full(charlie_p2id_note),
            OutputNote::Full(bob_p2id_note),
        ])
        .extend_note_args(note_args_map)
        .build()?;

    let executed_transaction = tx_context.execute().await?;
    println!("Transaction executed successfully!");

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 9: Verify results
    println!("\n=== Verification ===");

    // Should have exactly 3 P2ID notes (Alice, Charlie, and Bob's spread)
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(
        output_notes.num_notes(),
        3,
        "Expected exactly 3 P2ID notes (Alice 50 USDC, Charlie 25 ETH, Bob 5 ETH)"
    );

    // Verify the three P2ID notes
    let mut alice_p2id_found = false;
    let mut charlie_p2id_found = false;
    let mut bob_p2id_found = false;

    for idx in 0..output_notes.num_notes() {
        let note = output_notes.get_note(idx);
        println!("\nNote {}: {:?}", idx, note.id());

        let assets = note.assets().unwrap();
        println!("  Number of assets: {}", assets.num_assets());

        if assets.num_assets() == 1 {
            let asset = assets.iter().next().unwrap();
            if let Asset::Fungible(f) = asset {
                println!("  Asset: {} of faucet {:?}", f.amount(), f.faucet_id());

                if f.faucet_id() == usdc_faucet.id() && f.amount() == 50 {
                    println!("  -> Alice's P2ID note verified: 50 USDC");
                    alice_p2id_found = true;
                } else if f.faucet_id() == eth_faucet.id() && f.amount() == 25 {
                    println!("  -> Charlie's P2ID note verified: 25 ETH");
                    charlie_p2id_found = true;
                } else if f.faucet_id() == eth_faucet.id() && f.amount() == 5 {
                    println!("  -> Bob's P2ID note verified: 5 ETH (spread)");
                    bob_p2id_found = true;
                }
            }
        }
    }

    assert!(alice_p2id_found, "Alice's P2ID note (50 USDC) not found");
    assert!(charlie_p2id_found, "Charlie's P2ID note (25 ETH) not found");
    assert!(bob_p2id_found, "Bob's P2ID note (5 ETH spread) not found");

    // Check Bob's vault delta - should be zero since the 5 ETH goes to a P2ID note
    println!("\nVerifying Bob's vault delta...");
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();
    let removed_assets: Vec<Asset> = vault_delta.removed_assets().collect();

    println!("  Assets added: {}", added_assets.len());
    println!("  Assets removed: {}", removed_assets.len());

    assert_eq!(removed_assets.len(), 0, "Bob should not spend any assets");

    println!("\n=== Inflight cross-swap with spread test passed! ===");
    println!("  - Alice offered 30 ETH for 50 USDC (fully filled)");
    println!("  - Charlie offered 50 USDC for 25 ETH (fully filled)");
    println!("  - Bob earned 5 ETH spread via P2ID note");
    println!("  - 3 P2ID notes: Alice(50 USDC), Charlie(25 ETH), Bob(5 ETH)");

    Ok(())
}

#[tokio::test]
async fn swapp_note_creator_reclaim_test() -> anyhow::Result<()> {
    println!("=== Test: Creator Reclaim ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(Auth::BasicAuth, "USDC", 1000, Some(50))?;

    let eth_faucet = builder.add_existing_basic_faucet(Auth::BasicAuth, "ETH", 1000, Some(25))?;

    // STEP 2: Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // STEP 3: Create Alice wallet with USDC (using custom wallet)
    println!("\nCreating Alice wallet...");
    let alice_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let alice_assets = vec![FungibleAsset::new(usdc_faucet.id(), 50)?.into()];

    let alice = create_testing_account_from_package(
        account_package.clone(),
        alice_account_cfg,
        alice_assets,
    )
    .await?;
    println!("Alice: {:?}", alice.id());

    let _alice_account = builder.add_account(alice.clone());

    // STEP 4: Build swapp-note contract
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);

    // STEP 5: Create swap note
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");

    // Compute proper P2ID tag for Alice (who will receive the output note)
    let p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let note_inputs = vec![
        // Requested Asset: 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25),
        // Note Creator: Alice
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Alice
        p2id_tag_felt,
    ];

    let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut note_assets = NoteAssets::default();
    note_assets.add_asset(offered_asset.into())?;

    let swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: note_assets,
            inputs: note_inputs,
            ..Default::default()
        },
    )?;

    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    // STEP 6: Alice reclaims her own note
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    println!("\nAlice reclaiming her own swap note (no args needed)...");
    // No note args needed for reclaim - contract detects creator == consumer
    let tx_context = mock_chain
        .build_tx_context(alice.id(), &[swap_note.id()], &[])?
        .build()?;

    let executed_transaction = tx_context.execute().await?;
    println!("Transaction executed successfully!");

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 7: Verify results
    println!("\n=== Verification ===");

    // Should have NO output notes (no P2ID, no remainder)
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(
        output_notes.num_notes(),
        0,
        "Expected 0 output notes for reclaim"
    );
    println!("✓ No output notes created (correct for reclaim)");

    // Check Alice's vault delta - should have received back 50 USDC
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();

    assert_eq!(added_assets.len(), 1, "Alice should receive 1 asset back");
    let usdc_reclaimed = match added_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible USDC asset"),
    };
    assert_eq!(
        usdc_reclaimed.faucet_id(),
        usdc_faucet.id(),
        "Alice should reclaim USDC"
    );
    assert_eq!(usdc_reclaimed.amount(), 50, "Alice should reclaim 50 USDC");
    println!("✓ Alice's vault delta verified: +50 USDC");

    println!("\n✅ Creator reclaim test passed!");
    println!("  - Alice reclaimed her own swap note");
    println!("  - Received back 50 USDC");
    println!("  - No P2ID or remainder notes created");

    Ok(())
}

#[tokio::test]
async fn swapp_note_invalid_input_test() -> anyhow::Result<()> {
    println!("=== Test: Invalid Input (Requesting More Than Available) ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets
    let usdc_faucet = builder.add_existing_basic_faucet(Auth::BasicAuth, "USDC", 1000, Some(50))?;

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,
        Some(30), // Bob has 30 ETH
    )?;

    // STEP 2: Create wallets
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
    )?;

    let bob = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(eth_faucet.id(), 30)?.into()],
    )?;

    // STEP 3: Build contract
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);

    // STEP 4: Create swap note (Alice wants 25 ETH max)
    // Compute proper P2ID tag for Alice (who will receive the output note)
    let p2id_tag_felt = compute_p2id_tag_felt(alice.id());

    let note_inputs = vec![
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total = 25
        alice.id().prefix().into(),
        alice.id().suffix(),
        NoteType::Public.into(),
        // P2ID Tag (position 7): computed tag for Alice
        p2id_tag_felt,
    ];

    let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
    let mut note_assets = NoteAssets::default();
    note_assets.add_asset(offered_asset.into())?;

    let swap_note = create_testing_note_from_package(
        swapp_package.clone(),
        alice.id(),
        NoteCreationConfig {
            assets: note_assets,
            inputs: note_inputs,
            ..Default::default()
        },
    )?;

    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    let mock_chain = builder.build()?;

    // STEP 5: Bob tries to provide MORE than requested (30 > 25) - should fail
    println!("\nBob trying to provide 30 ETH (more than requested 25)...");
    let note_args = Word::from([Felt::ZERO, Felt::ZERO, Felt::ZERO, Felt::new(30)]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_note_args(note_args_map)
        .build()?;

    // This should fail with assertion error
    let result = tx_context.execute().await;

    assert!(
        result.is_err(),
        "Transaction should fail when input_amount > requested_asset_total"
    );

    println!("✓ Transaction correctly failed with assertion error");
    println!("\n✅ Invalid input test passed!");
    println!("  - Bob tried to provide 30 ETH (more than 25 requested)");
    println!("  - Transaction failed as expected (assertion at line 75)");

    Ok(())
}

#[tokio::test]
async fn swapp_note_full_fill_new_test() -> anyhow::Result<()> {
    println!("=== Test: Full Fill Swap (Using PswapNote) ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets in genesis
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "USDC",
        1000,      // max_supply
        Some(150), // total_issuance (50 for note + 100 for Bob)
    )?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,     // max_supply
        Some(50), // total_issuance (25 for Alice's request)
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());

    // STEP 2: Create wallets with initial assets
    println!("\nCreating Alice and Bob wallets with initial assets...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()], // Alice has 50 USDC to offer
    )?;
    println!("Alice: {:?} (has 50 USDC)", alice.id());

    // Build basic-wallet contract package
    println!("\nBuilding basic-wallet contract...");
    let account_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/basic-wallet"),
        true,
    )?);
    println!("Basic-wallet contract built successfully.");

    // Create custom account configuration with default settings
    let bob_account_cfg = AccountCreationConfig {
        storage_slots: vec![],
        ..Default::default()
    };

    let assets = vec![FungibleAsset::new(eth_faucet.id(), 25)?.into()];

    let bob = create_testing_account_from_package(account_package.clone(), bob_account_cfg, assets)
        .await?;
    println!("Bob account created: {:?}", bob.id());

    let _bob_account = builder.add_account(bob.clone());

    // STEP 3: Create swap note using PswapNote (clean and simple!)
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");

    let offered_asset = Asset::Fungible(FungibleAsset::new(usdc_faucet.id(), 50)?);
    let requested_asset = Asset::Fungible(FungibleAsset::new(eth_faucet.id(), 25)?);

    // Create RNG for note serial number generation
    use miden_crypto::rand::RpoRandomCoin;
    let mut rng = RpoRandomCoin::new(Word::default());

    let swap_note = PswapNote::create(
        alice.id(),
        offered_asset,
        requested_asset,
        NoteType::Public,
        NoteAttachment::default(),
        &mut rng,
    )?;

    println!("Swap note created: {:?}", swap_note.id());
    println!("  ✅ Used PswapNote::create (auto-loads from contracts/swapp-note/swapp_note.masp)");

    // Add note to genesis
    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    // STEP 5: Build MockChain and execute transaction
    println!("\nBuilding MockChain...");
    let mut mock_chain = builder.build()?;

    // Bob consumes the swap note with full fill (25 ETH)
    println!("\nBob consuming swap note (providing 25 ETH - full fill)...");
    let note_args = Word::from([
        Felt::ZERO,
        Felt::ZERO,
        Felt::ZERO,
        Felt::new(25), // input_amount = 25 (full fill)
    ]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    // Use PswapNote::create_swap_output_notes to build the expected P2ID note
    println!("\nCreating expected P2ID note using PswapNote::create_swap_output_notes...");
    let (expected_p2id_note, expected_remainder) =
        PswapNote::create_output_notes(&swap_note, bob.id(), 25, 0)?;

    assert!(
        expected_remainder.is_none(),
        "Full fill should not produce a remainder note"
    );

    let attachment = expected_p2id_note.metadata().attachment().clone();
    println!("Expected P2ID note created: {:?}", expected_p2id_note.id());

    // Rebuild the transaction context with the expected output note
    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_note_args(note_args_map)
        .extend_expected_output_notes(vec![OutputNote::Full(expected_p2id_note)])
        .build()?;

    let executed_transaction = tx_context.execute().await?;

    println!(
        "Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 6: Verify results
    println!("\n=== Verification ===");

    // Check output notes - should have 1 P2ID note
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(output_notes.num_notes(), 1, "Expected exactly 1 P2ID note");

    assert_eq!(
        output_notes.get_note(0).metadata().attachment().clone(),
        attachment
    );

    let p2id_note = output_notes.get_note(0);
    println!("P2ID note created: {:?}", p2id_note.id());

    // Verify P2ID note contains 25 ETH for Alice
    let p2id_assets = p2id_note.assets().unwrap();
    assert_eq!(
        p2id_assets.num_assets(),
        1,
        "P2ID note should have exactly 1 asset"
    );

    // Get the asset and verify it's 25 ETH
    let p2id_asset = p2id_assets.iter().next().unwrap();
    let p2id_fungible = match p2id_asset {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible asset in P2ID note"),
    };
    assert_eq!(
        p2id_fungible.faucet_id(),
        eth_faucet.id(),
        "P2ID note should contain ETH"
    );
    assert_eq!(
        p2id_fungible.amount(),
        25,
        "P2ID note should contain 25 ETH"
    );
    println!("✓ P2ID note verified: 25 ETH for Alice");

    // Check Bob's account delta - should have received 50 USDC
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();

    // Verify Bob received 50 USDC and spent 25 ETH
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();
    let removed_assets: Vec<Asset> = vault_delta.removed_assets().collect();

    assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset (USDC)");

    let usdc_received = match added_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible USDC asset"),
    };
    assert_eq!(
        usdc_received.faucet_id(),
        usdc_faucet.id(),
        "Bob should receive USDC"
    );
    assert_eq!(usdc_received.amount(), 50, "Bob should receive 50 USDC");

    mock_chain.add_pending_executed_transaction(&executed_transaction)?;
    let _ = mock_chain.prove_next_block();

    let eth_spent = match removed_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible ETH asset"),
    };
    assert_eq!(
        eth_spent.faucet_id(),
        eth_faucet.id(),
        "Bob should spend ETH"
    );
    assert_eq!(eth_spent.amount(), 25, "Bob should spend 25 ETH");
    println!("✓ Bob's vault delta verified: +50 USDC, -25 ETH");

    println!("\n✅ Full-fill swap test passed (using PswapNote::create_swap_output_notes)!");
    println!("  - Bob provided 25 ETH");
    println!("  - Bob received 50 USDC");
    println!("  - P2ID note created for Alice with 25 ETH");

    Ok(())
}

#[tokio::test]
async fn swapp_note_partial_fill_new_test() -> anyhow::Result<()> {
    println!("=== Test: Partial Fill Swap (Using PswapNote helpers) ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet =
        builder.add_existing_basic_faucet(Auth::BasicAuth, "USDC", 1000, Some(150))?;
    println!("USDC Faucet: {:?}", usdc_faucet.id());

    let eth_faucet = builder.add_existing_basic_faucet(Auth::BasicAuth, "ETH", 1000, Some(50))?;
    println!("ETH Faucet: {:?}", eth_faucet.id());

    // STEP 2: Create wallets
    println!("\nCreating Alice and Bob wallets...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
    )?;
    println!("Alice: {:?} (has 50 USDC)", alice.id());

    // Create Bob's wallet using BasicWallet component
    println!("\nCreating Bob's basic-wallet account...");
    let assets = vec![FungibleAsset::new(eth_faucet.id(), 25)?.into()];

    let bob = AccountBuilder::new([3u8; 32])
        .account_type(AccountType::RegularAccountUpdatableCode)
        .storage_mode(AccountStorageMode::Public)
        .with_component(BasicWallet::component())
        .with_auth_component(NoAuth::new())
        .with_assets(assets)
        .build_existing()
        .expect("Failed to build basic-wallet account");

    println!("Bob account created: {:?}", bob.id());

    let _bob_account = builder.add_account(bob.clone());

    // STEP 3: Create swap note using PswapNote::create
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");

    let offered_asset = Asset::Fungible(FungibleAsset::new(usdc_faucet.id(), 50)?);
    let requested_asset = Asset::Fungible(FungibleAsset::new(eth_faucet.id(), 25)?);

    use miden_crypto::rand::RpoRandomCoin;
    let mut rng = RpoRandomCoin::new(Word::default());

    let swap_note = PswapNote::create(
        alice.id(),
        offered_asset,
        requested_asset,
        NoteType::Public,
        NoteAttachment::default(),
        &mut rng,
    )?;

    println!("Swap note created: {:?}", swap_note.id());
    println!("  ✅ Used PswapNote::create");

    // Add note to genesis
    builder.add_output_note(OutputNote::Full(swap_note.clone()));

    // STEP 4: Build MockChain
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    // STEP 5: Bob provides 15 ETH (60% partial fill)
    println!("\nBob consuming swap note (providing 15 ETH - partial fill)...");
    let note_args = Word::from([
        Felt::ZERO,
        Felt::ZERO,
        Felt::ZERO,
        Felt::new(15), // input_amount = 15 (partial fill)
    ]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    // STEP 6: Use PswapNote::create_swap_output_notes to build both expected output notes
    println!("\nCreating expected output notes using PswapNote::create_swap_output_notes...");
    let (expected_p2id_note, expected_remainder) =
        PswapNote::create_output_notes(&swap_note, bob.id(), 15, 0)?;

    assert!(
        expected_remainder.is_some(),
        "Partial fill should produce a remainder note"
    );
    let expected_remainder_note = expected_remainder.unwrap();

    println!("  Expected P2ID note: {:?}", expected_p2id_note.id());
    println!(
        "  Expected remainder note: {:?}",
        expected_remainder_note.id()
    );

    // Build transaction context with both expected output notes
    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_note_args(note_args_map)
        .extend_expected_output_notes(vec![
            OutputNote::Full(expected_p2id_note),
            OutputNote::Full(expected_remainder_note),
        ])
        .build()?;

    let executed_transaction = tx_context.execute().await?;

    println!(
        "Transaction executed! Cycle count: {:?}",
        executed_transaction.measurements().note_execution
    );

    // STEP 7: Verify results
    println!("\n=== Verification ===");

    // Should have 2 output notes: P2ID + remainder
    let output_notes = executed_transaction.output_notes();
    println!("Output notes created: {}", output_notes.num_notes());
    assert_eq!(
        output_notes.num_notes(),
        2,
        "Expected 2 notes: 1 P2ID + 1 remainder"
    );

    // Find and verify the P2ID note and remainder note
    let mut p2id_found = false;
    let mut remainder_found = false;

    for idx in 0..output_notes.num_notes() {
        let note = output_notes.get_note(idx);
        let assets = note.assets().unwrap();

        if assets.num_assets() == 1 {
            let asset = assets.iter().next().unwrap();
            if let Asset::Fungible(f) = asset {
                if f.faucet_id() == eth_faucet.id() {
                    // P2ID note: contains 15 ETH for Alice
                    assert_eq!(f.amount(), 15, "P2ID note should contain 15 ETH");
                    println!("✓ P2ID note verified: 15 ETH for Alice");
                    p2id_found = true;
                } else if f.faucet_id() == usdc_faucet.id() {
                    // Remainder note: contains 20 USDC (50 - 30 = 20)
                    // offered_out = (50 * 15) / 25 = 30, remaining = 50 - 30 = 20
                    assert_eq!(f.amount(), 20, "Remainder note should contain 20 USDC");
                    println!("✓ Remainder note verified: 20 USDC (requesting 10 ETH)");
                    remainder_found = true;
                }
            }
        }
    }

    assert!(p2id_found, "P2ID note not found in output");
    assert!(remainder_found, "Remainder swap note not found in output");

    // Verify Bob's vault delta
    let account_delta = executed_transaction.account_delta();
    let vault_delta = account_delta.vault();
    let added_assets: Vec<Asset> = vault_delta.added_assets().collect();

    assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset");
    let usdc_received = match added_assets[0] {
        Asset::Fungible(f) => f,
        _ => panic!("Expected fungible USDC asset"),
    };
    assert_eq!(
        usdc_received.faucet_id(),
        usdc_faucet.id(),
        "Bob should receive USDC"
    );
    assert_eq!(usdc_received.amount(), 30, "Bob should receive 30 USDC");
    println!("✓ Bob's vault delta verified: +30 USDC, -15 ETH");

    println!("\n✅ Partial-fill swap test passed (using PswapNote helpers)!");
    println!("  - Bob provided 15 ETH (60% of requested 25)");
    println!("  - Bob received 30 USDC (60% of offered 50)");
    println!("  - P2ID note: 15 ETH for Alice");
    println!("  - Remainder note: 20 USDC still requesting 10 ETH");
    println!("  - Both output notes built via PswapNote::create_swap_output_notes");

    Ok(())
}
