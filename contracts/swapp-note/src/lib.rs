// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

#[macro_use]
extern crate alloc;

use crate::bindings::Account;
use alloc::vec::Vec;
use miden::*;

/// Swapp Note Script
///
/// Implements a partially-fillable swap note for DEX functionality.
///
/// **Note Arg (via `arg` parameter - provided by note consumer):**
/// - Position 0: input_amount: Felt (single Felt value for amount)
/// - Position 1: inflight_amount: Felt (single Felt value for amount)
/// - Position 2: 0: Felt (unused)
/// - Position 3: 0: Felt (unused)
/// arg structure: [input_amount, inflight_amount, 0, 0]
///
/// **Note Inputs (via `active_note::get_inputs()` - stored when note is created):**
/// - Positions 0-3: Requested Asset Word (4 Felts)
///   - inputs[0]: requested_asset_id_prefix (Felt)
///   - inputs[1]: requested_asset_id_suffix (Felt)
///   - inputs[2]: padding (0, Felt)
///   - inputs[3]: requested_asset_total (Felt)
/// - Positions 4-7: Note Creator AccountId (4 Felts)
///   - inputs[4]: note_creator_account_id_prefix (Felt)
///   - inputs[5]: note_creator_account_id_suffix (Felt)
///   - inputs[6]: padding (0, Felt)
///   - inputs[7]: padding (0, Felt)
///
///
#[note_script]
fn run(arg: Word, account: &mut Account) {
    // Get stored note inputs
    let inputs = active_note::get_inputs();

    // Get executing account ID (the note consumer)
    let executing_account_id = active_account::get_id();
    let swapp_note_creator_id = AccountId::from(inputs[4], inputs[5]);

    if swapp_note_creator_id == executing_account_id {
        // Note creator is consuming their own note - receive assets back
        // Moves all assets from the note into the executing account's vault
        active_note::add_assets_to_account();
        return;
    }

    active_note::add_assets_to_account();

    // Extract input_amount from note arg (provided by note consumer)
    let input_amount = arg[0];
    let inflight_amount = arg[1];
    let total_input_amount = input_amount + inflight_amount;

    // Validate that offered_asset_word matches the asset in the active note
    let note_assets = active_note::get_assets();

    // Check that there is exactly one asset in the note
    let num_assets = note_assets.len();
    assert_eq(Felt::from_u32(num_assets as u32), felt!(1));

    // Get the asset from the note
    let offered_asset = note_assets[0];

    // Extract amounts for calculations
    let requested_asset_total = inputs[3];
    let offered_asset_total = offered_asset.inner[0];

    // Get the current note serial number
    let current_note_serial = active_note::get_serial_number();

    {
        // Validate input: input_amount must not exceed requested_asset_total
        let is_valid = if total_input_amount.as_u64() <= requested_asset_total.as_u64() {
            felt!(1)
        } else {
            felt!(0)
        };

        assert_eq(is_valid, felt!(1));
    }

    // Compute offered output amount proportional to input
    let input_offered_out =
        calculate_output_amount(offered_asset_total, requested_asset_total, input_amount);

    let inflight_offered_out =
        calculate_output_amount(offered_asset_total, requested_asset_total, inflight_amount);

    assert_eq!(current_note_serial.inner.3, felt!(0));

    // Create routing (P2ID) note, this is the note that will be used to route the requested asset to the note creator
    let routing_serial = add_word(
        current_note_serial,
        Word::from([felt!(1), felt!(1), felt!(1), felt!(1)]),
    );

    let total_input_amount = input_amount + inflight_amount;
    // aux value is the input amount so that the swapp note creator can determine build the note
    let aux_value = total_input_amount;
    let input_asset = Asset::new(Word::from([inputs[0], inputs[1], inputs[2], input_amount]));

    // Create P2ID note using output_note module
    let p2id_note_idx = create_p2id_note(
        routing_serial,
        input_asset,
        swapp_note_creator_id,
        aux_value,
        account,
    );

    let inputs = active_note::get_inputs();
    let inflight_amount = arg[1];

    // Add the inflight amount to the p2id note
    let inflight_asset = Asset::new(Word::from([
        inputs[0],
        inputs[1],
        inputs[2],
        inflight_amount,
    ]));
    let inflight_asset_reversed = Asset::new(inflight_asset.inner.reverse());
    output_note::add_asset(inflight_asset_reversed, p2id_note_idx);

    // Compute the total input amount( Need to recalculate this value because earlier one got of stack memory )
    let total_input_amount = input_amount + inflight_amount;
    let total_offered_out = input_offered_out + inflight_offered_out;

    // Create remainder swap note in case of partial fill
    if total_offered_out.as_u64() < offered_asset_total.as_u64() {
        let remainder_serial = hash_words(&[current_note_serial]).inner;
        let remainder_aux = total_offered_out;
        let requested_asset_total = inputs[3] - total_input_amount;
        let remainder_requested_asset =
            Asset::from([inputs[0], inputs[1], inputs[2], requested_asset_total]);

        let remainder_offered_asset_total = offered_asset_total - total_offered_out;
        let remainder_offered_asset = Asset::new(Word::from([
            offered_asset.inner[3],
            offered_asset.inner[2],
            offered_asset.inner[1],
            remainder_offered_asset_total,
        ]));

        let padded_inputs = vec![
            remainder_requested_asset.inner[0],
            remainder_requested_asset.inner[1],
            remainder_requested_asset.inner[2],
            remainder_requested_asset.inner[3],
            swapp_note_creator_id.prefix,
            swapp_note_creator_id.suffix,
            felt!(0),
            felt!(0),
        ];

        create_swapp_note(
            remainder_serial,
            remainder_aux,
            remainder_offered_asset,
            padded_inputs,
            account,
        );
    }
}

///
/// # Arguments
/// * `offered_total` - Total offered asset amount (Felt)
/// * `requested_total` - Total requested asset amount (Felt)
/// * `input_amount` - Input asset amount provided (Felt)
///
/// # Returns
/// Output asset amount proportional to the ratio (Felt)
fn calculate_output_amount(offered_total: Felt, requested_total: Felt, input_amount: Felt) -> Felt {
    let precision_factor = Felt::from_u32(100000);

    // For the better precision, we use the two different paths for the calculation
    if offered_total.as_u64() > requested_total.as_u64() {
        // Case 1: offered_total > requested_total
        // Calculate ratio = (offered_total * factor) / requested_total
        // Then output = (input_amount * ratio) / factor
        let ratio = (offered_total * precision_factor) / requested_total;
        return (input_amount * ratio) / precision_factor;
    } else {
        // Case 2: offered_total <= requested_total
        // Direct calculation: (input_amount * offered_total * factor) / (requested_total * factor)
        let ratio = (requested_total * precision_factor) / offered_total;
        return (input_amount * precision_factor) / ratio;
    }
}

/// Add two Words element-wise
fn add_word(a: Word, b: Word) -> Word {
    Word::from([a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]])
}

/// Create a P2ID (Pay-to-ID) note
fn create_p2id_note(
    serial_num: Word,
    input_asset: Asset,
    recipient_id: AccountId,
    aux: Felt,
    account: &mut Account,
) -> NoteIdx {
    // Create a tag for the P2ID note - LocalAny with payload 0
    // This equals NoteTag::LocalAny(0) in the SDK, which serializes to 0xC0000000
    let tag = Tag::from(Felt::from_u32(0xC0000000));

    // Create a same note type as the active note
    //let note_type = get_note_type();
    let note_type = NoteType::from(felt!(1));

    // Set execution hint (always executable for now)
    let execution_hint = felt!(0);

    let p2id_note_root_digest = Digest::from_word(Word::new([
        Felt::from_u64_unchecked(15783632360113277539),
        Felt::from_u64_unchecked(7403765918285273520),
        Felt::from_u64_unchecked(15691985194755641846),
        Felt::from_u64_unchecked(10399643920503194563),
    ]));

    // Create recipient from serial number and account ID
    let recipient = Recipient::compute(
        serial_num,
        p2id_note_root_digest,
        vec![
            recipient_id.suffix,
            recipient_id.prefix,
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
        ],
    );

    assert_eq!(
        recipient.inner[0],
        Felt::from_u64_unchecked(4680597347679157214)
    );
    assert_eq!(
        recipient.inner[1],
        Felt::from_u64_unchecked(1254494794927659471)
    );
    assert_eq!(
        recipient.inner[2],
        Felt::from_u64_unchecked(15467102009091135960)
    );
    assert_eq!(
        recipient.inner[3],
        Felt::from_u64_unchecked(11428255931367355774)
    );

    // Create the note using output_note::create
    let note_idx = output_note::create(tag, aux, note_type, execution_hint, recipient);

    assert_eq(input_asset.inner[3], felt!(0));
    if input_asset.inner[3] != felt!(0) {
        let input_asset_reversed = Asset::new(input_asset.inner.reverse());

        account.move_asset_to_note(input_asset_reversed, note_idx)
    }

    return note_idx;
}
/// Create a Swapp note with remainder parameters
fn create_swapp_note(
    serial_num: Word,
    aux: Felt,
    offered_asset: Asset,
    padded_inputs: Vec<Felt>,
    account: &mut Account,
) {
    // Create a tag for the P2ID note - LocalAny with payload 0
    // This equals NoteTag::LocalAny(0) in the SDK, which serializes to 0xC0000000
    let tag = Tag::from(Felt::from_u32(0xC0000000));

    // Create a same note type as the active note
    //let note_type = get_note_type();
    let note_type = NoteType::from(felt!(1));

    // Set execution hint (always executable for now)
    let execution_hint = felt!(0);

    // Create recipient with swapp script and remainder parameters
    let recipient = Recipient::compute(
        serial_num,
        Digest::from_word(active_note::get_script_root()),
        padded_inputs,
    );

    // Create the note using output_note::create
    let note_idx = output_note::create(tag, aux, note_type, execution_hint, recipient);

    let offered_asset_reversed = Asset::new(offered_asset.inner.reverse());

    // Add the asset to the note
    account.move_asset_to_note(offered_asset_reversed, note_idx);
}

fn get_note_tag() -> Tag {
    let metadata = active_note::get_metadata();
    // Shift left by 32 bits
    let left_shifted_32 = metadata[2] * Felt::from_u32(2u32.pow(32));
    // Shift right by 32 bits
    let tag_felt = left_shifted_32 / (Felt::from_u32(2u32.pow(32)));
    Tag::from(tag_felt)
}

fn get_note_type() -> NoteType {
    let metadata = active_note::get_metadata();
    // 2nd felt: [sender_id_suffix (56 bits) | note_type (2 bits) | note_execution_hint_tag (6 bits)]
    // Extract note_type: shift right by 6 bits (to skip note_execution_hint_tag), then mask with 0b11 (2 bits)
    let second_felt = metadata[2];

    // Shift left by 56 bits
    let left_shifted_56 = second_felt * Felt::from_u64_unchecked(2u64.pow(56));
    // Shift right by 62
    let note_type_felt = left_shifted_56 / Felt::from_u64_unchecked(2u64.pow(62));
    NoteType::from(note_type_felt)
}
