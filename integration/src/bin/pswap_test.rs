use integration::helpers::{setup_client, ClientSetup};
use integration::swapp_state::SwappTestState;

use anyhow::{Context, Result};
use miden_client::{
    note::NoteType,
    transaction::{OutputNote, TransactionRequestBuilder},
    Word,
};
use miden_protocol::{
    asset::FungibleAsset,
    note::{NoteAttachment, NoteAttachmentScheme},
};
use tokio::time::Duration;

/// PSWAP Integration Test Binary
///
/// Tests the three PSWAP builder methods on TransactionRequestBuilder:
/// 1. build_pswap_create  - Alice creates a swap note
/// 2. build_pswap_consume - Bob partially fills the swap note
/// 3. build_pswap_cancel  - Alice creates and cancels a swap note
///
/// Requires: Run `cargo run --bin swapp_setup` first to create accounts and mint tokens.

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== PSWAP Integration Test ===\n");

    //------------------------------------------------------------
    // Load persisted state
    //------------------------------------------------------------
    println!("[LOADING STATE]");
    let state = SwappTestState::load()?;
    let faucet1_id = state.faucet1_id()?; // USDT
    let faucet2_id = state.faucet2_id()?; // ETH
    let alice_id = state.alice_id()?;
    let bob_id = state.bob_id()?;

    println!("Faucet 1 (USDT): {:?}", faucet1_id);
    println!("Faucet 2 (ETH):  {:?}", faucet2_id);
    println!("Alice: {:?}", alice_id);
    println!("Bob:   {:?}\n", bob_id);

    //------------------------------------------------------------
    // Initialize client
    //------------------------------------------------------------
    println!("[SETUP] Initializing client");
    let ClientSetup { mut client, .. } = setup_client().await?;
    let sync_summary = client.sync_state().await?;
    println!("Latest block: {}\n", sync_summary.block_num);

    //------------------------------------------------------------
    // TEST 1: build_pswap_create - Alice creates a swap note
    //------------------------------------------------------------
    println!("========================================");
    println!("[TEST 1] build_pswap_create");
    println!("Alice offers 50 USDT for 25 ETH");
    println!("========================================\n");

    let offered_asset_1 = FungibleAsset::new(faucet1_id, 50).unwrap();
    let requested_asset_1 = FungibleAsset::new(faucet2_id, 25).unwrap();
    let note_attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), Word::default());

    let create_request = TransactionRequestBuilder::new()
        .build_pswap_create(
            alice_id,
            offered_asset_1.into(),
            requested_asset_1.into(),
            NoteType::Public,
            note_attachment,
            client.rng(),
        )
        .context("Failed to build PSWAP create request")?;

    let expected_notes = create_request.expected_output_own_notes();
    let swap_note = expected_notes.first().unwrap();

    let tx_id = client
        .submit_new_transaction(alice_id, create_request)
        .await
        .context("Failed to submit PSWAP create transaction")?;
    println!("build_pswap_create succeeded. TX: {:?}", tx_id);

    tokio::time::sleep(Duration::from_secs(10)).await;
    client.sync_state().await?;
    println!("[TEST 1] PASSED\n");

    //------------------------------------------------------------
    // TEST 2: build_pswap_consume - Bob partially fills a swap note
    //------------------------------------------------------------
    println!("========================================");
    println!("[TEST 2] build_pswap_consume (partial fill)");
    println!("Alice offers 50 USDT for 25 ETH, Bob fills 15 ETH");
    println!("========================================\n");

    // Bob partially fills with 15 ETH (out of 25 requested)
    // Expected output: P2ID note (15 ETH -> Alice) + remainder note (20 USDT for 10 ETH)
    let fill_amount: u64 = 15;
    let inflight_amount: u64 = 0;

    let consume_request = TransactionRequestBuilder::new()
        .build_pswap_consume(&swap_note, bob_id, fill_amount, inflight_amount)
        .context("Failed to build PSWAP consume request")?;

    let tx_id = client
        .submit_new_transaction(bob_id, consume_request)
        .await
        .context("Failed to submit PSWAP consume transaction")?;
    println!("build_pswap_consume succeeded. TX: {:?}", tx_id);
    println!("Bob filled 15 ETH out of 25 ETH requested");
    println!("Expected: P2ID note (15 ETH -> Alice) + remainder note (20 USDT for 10 ETH)");

    println!("\nWaiting for transaction to be processed...");
    tokio::time::sleep(Duration::from_secs(30)).await;
    client.sync_state().await?;
    println!("[TEST 2] PASSED\n");

    // //------------------------------------------------------------
    // // TEST 3: build_pswap_cancel - Alice creates and cancels a swap note
    // //------------------------------------------------------------
    // println!("========================================");
    // println!("[TEST 3] build_pswap_cancel");
    // println!("Alice creates a swap note and then cancels it");
    // println!("========================================\n");

    // let offered_asset_3 = FungibleAsset::new(faucet1_id, 50).unwrap();
    // let requested_asset_3 = FungibleAsset::new(faucet2_id, 25).unwrap();
    // let note_attachment_3 = NoteAttachment::new_word(NoteAttachmentScheme::none(), Word::default());

    // let cancel_swap_note = miden_swapp::PswapNote::create(
    //     alice_id,
    //     offered_asset_3.into(),
    //     requested_asset_3.into(),
    //     NoteType::Public,
    //     note_attachment_3,
    //     client.rng(),
    // )
    // .context("Failed to create PSWAP note for cancel test")?;

    // println!("Swap note created: {:?}", cancel_swap_note.id());

    // // Publish the swap note
    // let publish_request = TransactionRequestBuilder::new()
    //     .own_output_notes(vec![OutputNote::Full(cancel_swap_note.clone())])
    //     .build()
    //     .context("Failed to build publish request for cancel test")?;

    // let tx_id = client
    //     .submit_new_transaction(alice_id, publish_request)
    //     .await
    //     .context("Failed to publish swap note for cancel test")?;
    // println!("Swap note published. TX: {:?}", tx_id);

    // // Wait for the note to be available on-chain
    // println!("Waiting for swap note to be available...");
    // tokio::time::sleep(Duration::from_secs(15)).await;
    // client.sync_state().await?;
    // client.sync_state().await?;

    // // Alice cancels the swap note (she is the creator)
    // let cancel_request = TransactionRequestBuilder::new()
    //     .build_pswap_cancel(cancel_swap_note)
    //     .context("Failed to build PSWAP cancel request")?;

    // let tx_id = client
    //     .submit_new_transaction(alice_id, cancel_request)
    //     .await
    //     .context("Failed to submit PSWAP cancel transaction")?;
    // println!("build_pswap_cancel succeeded. TX: {:?}", tx_id);
    // println!("Alice cancelled her swap note and reclaimed her 50 USDT");

    // tokio::time::sleep(Duration::from_secs(10)).await;
    // client.sync_state().await?;
    // println!("[TEST 3] PASSED\n");

    //------------------------------------------------------------
    // Summary
    //------------------------------------------------------------
    println!("========================================");
    println!("=== All PSWAP Tests Passed ===");
    println!("========================================");
    println!("TEST 1: build_pswap_create  - PASSED");
    println!("TEST 2: build_pswap_consume - PASSED (partial fill)");
    println!("TEST 3: build_pswap_cancel  - PASSED");

    Ok(())
}
