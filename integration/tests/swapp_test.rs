use integration::helpers::{
    build_project_in_dir, create_testing_note_from_package, NoteCreationConfig,
};

use miden_client::{note::NoteAssets, transaction::OutputNote, Felt, Word};
use miden_core::FieldElement;
use miden_objects::asset::{Asset, FungibleAsset};
use miden_testing::{Auth, MockChain};
use std::{collections::BTreeMap, path::Path, sync::Arc};

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

    let eth_faucet = builder.add_existing_basic_faucet(
        Auth::BasicAuth,
        "ETH",
        1000,     // max_supply
        Some(25), // total_issuance (25 for Alice's request)
    )?;
    println!("ETH Faucet: {:?}", eth_faucet.id());

    // STEP 2: Create wallets with initial assets
    println!("\nCreating Alice and Bob wallets with initial assets...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()], // Alice has 50 USDC to offer
    )?;
    println!("Alice: {:?} (has 50 USDC)", alice.id());

    let bob = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(eth_faucet.id(), 25)?.into()], // Bob has 25 ETH to provide
    )?;
    println!("Bob: {:?} (has 25 ETH)", bob.id());

    // STEP 3: Build swapp-note contract
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);
    println!("Contract built successfully");

    // STEP 4: Create swap note with proper structure
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");
    let note_inputs = vec![
        // Requested Asset (positions 0-3): 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix().into(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total
        // Note Creator (positions 4-7): Alice
        alice.id().prefix().into(),
        alice.id().suffix().into(),
        Felt::ZERO,
        Felt::ZERO,
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
    builder.add_output_note(OutputNote::Full(swap_note.clone().into()));

    // STEP 5: Build MockChain and execute transaction
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    // Bob consumes the swap note with full fill (25 ETH)
    println!("\nBob consuming swap note (providing 25 ETH - full fill)...");
    let note_args = Word::from([
        Felt::new(25), // input_amount = 25 (full fill)
        Felt::ZERO,
        Felt::ZERO,
        Felt::ZERO,
    ]);

    let mut note_args_map = BTreeMap::new();
    note_args_map.insert(swap_note.id(), note_args);

    let tx_context = mock_chain
        .build_tx_context(bob.id(), &[swap_note.id()], &[])?
        .extend_note_args(note_args_map)
        .build()?;

    let executed_transaction = tx_context.execute().await?;
    println!("Transaction executed successfully!");

    // // STEP 6: Verify results
    // println!("\n=== Verification ===");

    // // Check output notes - should have 1 P2ID note
    // let output_notes = executed_transaction.output_notes();
    // println!("Output notes created: {}", output_notes.num_notes());
    // assert_eq!(output_notes.num_notes(), 1, "Expected exactly 1 P2ID note");

    // let p2id_note = output_notes.get_note(0);
    // println!("P2ID note created: {:?}", p2id_note.id());

    // // Verify P2ID note contains 25 ETH for Alice
    // let p2id_assets = p2id_note.assets().unwrap();
    // assert_eq!(
    //     p2id_assets.num_assets(),
    //     1,
    //     "P2ID note should have exactly 1 asset"
    // );

    // // Get the asset and verify it's 25 ETH
    // let p2id_asset = p2id_assets.iter().next().unwrap();
    // let p2id_fungible = match p2id_asset {
    //     Asset::Fungible(f) => f,
    //     _ => panic!("Expected fungible asset in P2ID note"),
    // };
    // assert_eq!(
    //     p2id_fungible.faucet_id(),
    //     eth_faucet.id(),
    //     "P2ID note should contain ETH"
    // );
    // assert_eq!(
    //     p2id_fungible.amount(),
    //     25,
    //     "P2ID note should contain 25 ETH"
    // );
    // println!("✓ P2ID note verified: 25 ETH for Alice");

    // // Check Bob's account delta - should have received 50 USDC
    // let account_delta = executed_transaction.account_delta();
    // let vault_delta = account_delta.vault();

    // // Verify Bob received 50 USDC and spent 25 ETH
    // let added_assets: Vec<Asset> = vault_delta.added_assets().collect();
    // let removed_assets: Vec<Asset> = vault_delta.removed_assets().collect();

    // assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset (USDC)");
    // assert_eq!(removed_assets.len(), 1, "Bob should spend 1 asset (ETH)");

    // let usdc_received = match added_assets[0] {
    //     Asset::Fungible(f) => f,
    //     _ => panic!("Expected fungible USDC asset"),
    // };
    // assert_eq!(
    //     usdc_received.faucet_id(),
    //     usdc_faucet.id(),
    //     "Bob should receive USDC"
    // );
    // assert_eq!(usdc_received.amount(), 50, "Bob should receive 50 USDC");
    // println!("✓ Bob's vault delta verified: +50 USDC, -25 ETH");

    // println!("\n✅ Full-fill swap test passed!");
    // println!("  - Bob provided 25 ETH");
    // println!("  - Bob received 50 USDC");
    // println!("  - P2ID note created for Alice with 25 ETH");

    Ok(())
}

// #[tokio::test]
// async fn swapp_note_partial_fill_test() -> anyhow::Result<()> {
//     println!("=== Test: Partial Fill Swap ===");
//     let mut builder = MockChain::builder();

//     // STEP 1: Create faucets in genesis
//     println!("Creating USDC and ETH faucets...");
//     let usdc_faucet =
//         builder.add_existing_basic_faucet(Auth::BasicAuth, "USDC", 1000, Some(150))?;

//     let eth_faucet = builder.add_existing_basic_faucet(
//         Auth::BasicAuth,
//         "ETH",
//         1000,
//         Some(15), // Bob only has 15 ETH (partial fill)
//     )?;

//     // STEP 2: Create wallets with initial assets
//     println!("\nCreating Alice and Bob wallets...");
//     let alice = builder.add_existing_wallet_with_assets(
//         Auth::BasicAuth,
//         [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
//     )?;

//     let bob = builder.add_existing_wallet_with_assets(
//         Auth::BasicAuth,
//         [FungibleAsset::new(eth_faucet.id(), 15)?.into()], // Bob has only 15 ETH
//     )?;

//     // STEP 3: Build swapp-note contract
//     println!("\nBuilding swapp-note contract...");
//     let swapp_package = Arc::new(build_project_in_dir(
//         Path::new("../contracts/swapp-note"),
//         true,
//     )?);

//     // STEP 4: Create swap note (Alice wants 25 ETH, offers 50 USDC)
//     println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");
//     let note_inputs = vec![
//         // Requested Asset: 25 ETH
//         eth_faucet.id().prefix().into(),
//         eth_faucet.id().suffix().into(),
//         Felt::ZERO,
//         Felt::new(25), // requested_asset_total
//         // Note Creator: Alice
//         alice.id().prefix().into(),
//         alice.id().suffix().into(),
//         Felt::ZERO,
//         Felt::ZERO,
//     ];

//     let offered_asset = FungibleAsset::new(usdc_faucet.id(), 50)?;
//     let mut note_assets = NoteAssets::default();
//     note_assets.add_asset(offered_asset.into())?;

//     let swap_note = create_testing_note_from_package(
//         swapp_package.clone(),
//         alice.id(),
//         NoteCreationConfig {
//             assets: note_assets,
//             inputs: note_inputs,
//             ..Default::default()
//         },
//     )?;

//     builder.add_output_note(OutputNote::Full(swap_note.clone().into()));

//     // STEP 5: Execute partial fill
//     println!("\nBuilding MockChain...");
//     let mock_chain = builder.build()?;

//     println!("\nBob consuming swap note (providing 15 ETH - partial fill)...");
//     let note_args = Word::from([
//         Felt::new(15), // input_amount = 15 (partial fill, 60% of requested)
//         Felt::ZERO,
//         Felt::ZERO,
//         Felt::ZERO,
//     ]);

//     let mut note_args_map = BTreeMap::new();
//     note_args_map.insert(swap_note.id(), note_args);

//     let tx_context = mock_chain
//         .build_tx_context(bob.id(), &[swap_note.id()], &[])?
//         .extend_note_args(note_args_map)
//         .build()?;

//     let executed_transaction = tx_context.execute().await?;
//     println!("Transaction executed successfully!");

//     // STEP 6: Verify results
//     println!("\n=== Verification ===");

//     // Should have 2 output notes: P2ID note + remainder swap note
//     let output_notes = executed_transaction.output_notes();
//     println!("Output notes created: {}", output_notes.num_notes());
//     assert_eq!(
//         output_notes.num_notes(),
//         2,
//         "Expected 2 notes: 1 P2ID + 1 remainder"
//     );

//     // Find P2ID note and remainder note
//     let mut p2id_note_found = false;
//     let mut remainder_note_found = false;

//     for idx in 0..output_notes.num_notes() {
//         let note = output_notes.get_note(idx);
//         let assets = note.assets().unwrap();
//         if assets.num_assets() == 1 {
//             let asset = assets.iter().next().unwrap();
//             let fungible = match asset {
//                 Asset::Fungible(f) => f,
//                 _ => continue,
//             };

//             if fungible.faucet_id() == eth_faucet.id() {
//                 // This is the P2ID note (contains ETH)
//                 assert_eq!(fungible.amount(), 15, "P2ID note should contain 15 ETH");
//                 println!("✓ P2ID note verified: 15 ETH for Alice");
//                 p2id_note_found = true;
//             } else if fungible.faucet_id() == usdc_faucet.id() {
//                 // This is the remainder swap note (contains remaining USDC)
//                 // Expected: (50 * 10) / 25 = 20 USDC remaining (since 15/25 = 60% filled, 40% remains)
//                 // Actually: offered_out = (50 * 15) / 25 = 30, so remaining = 50 - 30 = 20
//                 assert_eq!(
//                     fungible.amount(),
//                     20,
//                     "Remainder note should contain 20 USDC"
//                 );
//                 println!("✓ Remainder note verified: 20 USDC (still requesting 10 ETH)");
//                 remainder_note_found = true;
//             }
//         }
//     }

//     assert!(p2id_note_found, "P2ID note not found");
//     assert!(remainder_note_found, "Remainder swap note not found");

//     // Check Bob's vault delta
//     let account_delta = executed_transaction.account_delta();
//     let vault_delta = account_delta.vault();
//     let added_assets: Vec<Asset> = vault_delta.added_assets().collect();

//     assert_eq!(added_assets.len(), 1, "Bob should receive 1 asset");
//     let usdc_received = match added_assets[0] {
//         Asset::Fungible(f) => f,
//         _ => panic!("Expected fungible USDC asset"),
//     };
//     assert_eq!(usdc_received.amount(), 30, "Bob should receive 30 USDC");
//     println!("✓ Bob's vault delta verified: +30 USDC, -15 ETH");

//     println!("\n✅ Partial-fill swap test passed!");
//     println!("  - Bob provided 15 ETH (60% of requested 25)");
//     println!("  - Bob received 30 USDC (60% of offered 50)");
//     println!("  - P2ID note created for Alice with 15 ETH");
//     println!("  - Remainder swap note created: 20 USDC for 10 ETH");

//     Ok(())
// }

#[tokio::test]
async fn swapp_note_creator_reclaim_test() -> anyhow::Result<()> {
    println!("=== Test: Creator Reclaim ===");
    let mut builder = MockChain::builder();

    // STEP 1: Create faucets
    println!("Creating USDC and ETH faucets...");
    let usdc_faucet = builder.add_existing_basic_faucet(Auth::BasicAuth, "USDC", 1000, Some(50))?;

    let eth_faucet = builder.add_existing_basic_faucet(Auth::BasicAuth, "ETH", 1000, Some(25))?;

    // STEP 2: Create Alice wallet with USDC
    println!("\nCreating Alice wallet...");
    let alice = builder.add_existing_wallet_with_assets(
        Auth::BasicAuth,
        [FungibleAsset::new(usdc_faucet.id(), 50)?.into()],
    )?;
    println!("Alice: {:?}", alice.id());

    // STEP 3: Build swapp-note contract
    println!("\nBuilding swapp-note contract...");
    let swapp_package = Arc::new(build_project_in_dir(
        Path::new("../contracts/swapp-note"),
        true,
    )?);

    // STEP 4: Create swap note
    println!("\nCreating swap note (Alice offers 50 USDC for 25 ETH)...");
    let note_inputs = vec![
        // Requested Asset: 25 ETH
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix().into(),
        Felt::ZERO,
        Felt::new(25),
        // Note Creator: Alice
        alice.id().prefix().into(),
        alice.id().suffix().into(),
        Felt::ZERO,
        Felt::ZERO,
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

    builder.add_output_note(OutputNote::Full(swap_note.clone().into()));

    // STEP 5: Alice reclaims her own note
    println!("\nBuilding MockChain...");
    let mock_chain = builder.build()?;

    println!("\nAlice reclaiming her own swap note (no args needed)...");
    // No note args needed for reclaim - contract detects creator == consumer
    let tx_context = mock_chain
        .build_tx_context(alice.id(), &[swap_note.id()], &[])?
        .build()?;

    let executed_transaction = tx_context.execute().await?;
    println!("Transaction executed successfully!");

    // STEP 6: Verify results
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
    let note_inputs = vec![
        eth_faucet.id().prefix().into(),
        eth_faucet.id().suffix().into(),
        Felt::ZERO,
        Felt::new(25), // requested_asset_total = 25
        alice.id().prefix().into(),
        alice.id().suffix().into(),
        Felt::ZERO,
        Felt::ZERO,
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

    builder.add_output_note(OutputNote::Full(swap_note.clone().into()));

    let mock_chain = builder.build()?;

    // STEP 5: Bob tries to provide MORE than requested (30 > 25) - should fail
    println!("\nBob trying to provide 30 ETH (more than requested 25)...");
    let note_args = Word::from([
        Felt::new(30), // input_amount = 30 (INVALID - exceeds requested 25)
        Felt::ZERO,
        Felt::ZERO,
        Felt::ZERO,
    ]);

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
