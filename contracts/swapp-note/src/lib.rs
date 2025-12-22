// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

#[macro_use]
extern crate alloc;

use miden::*;

/// Swapp Note Script
///
/// Implements a partially-fillable limit order for DEX functionality.
/// Based on the Miden SDK: https://docs.rs/miden/latest/miden/
///
/// **Note Arg (via `arg` parameter - provided by note consumer):**
/// - Position 0: inflight: bool (1 Felt value: 0 or 1)
/// - Position 1: input_amount: Felt (single Felt value for amount)
/// - Position 2: reserved (unused)
/// - Position 3: reserved (unused)
/// arg structure: [inflight, amount, reserved, reserved]
///
/// **Note Inputs (via `active_note::get_inputs()` - stored when note is created):**
/// - Positions 0-3: Requested Asset Word (4 Felts)
///   - inputs[0]: requested_asset_id_prefix (Felt)
///   - inputs[1]: requested_asset_id_suffix (Felt)
///   - inputs[2]: padding (0, Felt)
///   - inputs[3]: requested_asset_total (Felt)
/// - Positions 4-7: Offered Asset Word (4 Felts)
///   - inputs[4]: offered_asset_id_prefix (Felt)
///   - inputs[5]: offered_asset_id_suffix (Felt)
///   - inputs[6]: padding (0, Felt)
///   - inputs[7]: offered_asset_total (Felt)
/// - Positions 8-11: Note Creator AccountId (4 Felts)
///   - inputs[8]: note_creator_account_id_prefix (Felt)
///   - inputs[9]: note_creator_account_id_suffix (Felt)
///   - inputs[10]: padding (0, Felt)
///   - inputs[11]: padding (0, Felt)
#[note_script]
fn run(arg: Word) {
    // Get stored note inputs
    let inputs = active_note::get_inputs();

    // Step 1: Get executing account ID
    // The note is consumed by the executing account, which is implicitly the recipient
    let executing_account_id = active_account::get_id();
    let swapp_note_creator_id = AccountId::from(inputs[8], inputs[9]);

    if swapp_note_creator_id == executing_account_id {
        // Note creator is consuming their own note - receive assets back
        // Moves all assets from the note into the executing account's vault
        active_note::add_assets_to_account();
        return;
    }

    // Step 2: Extract input_amount and inflight from note arg
    // These are provided by the note consumer when executing the note
    // arg structure: [inflight, amount, reserved, reserved]
    let inflight_val = arg[0];
    let input_amount = arg[1];
    let inflight = inflight_val != felt!(0);

    // Step 3: Extract Asset Words from note inputs (stored when note was created)
    // Each Asset Word = 4 Felts: [asset_id_prefix, asset_id_suffix, 0, amount]

    // Create Asset Words directly from inputs
    // Requested Asset Word (positions 0-3)
    let _requested_asset_word = Asset::from([
        inputs[0], // Felt[0] - requested_asset_id_prefix
        inputs[1], // Felt[1] - requested_asset_id_suffix
        inputs[2], // Felt[2] - padding (should be 0)
        inputs[3], // Felt[3] - requested_asset_total
    ]);

    // Offered Asset Word (positions 4-7)
    let offered_asset_word = Asset::from([
        inputs[4], // Felt[0] - offered_asset_id_prefix
        inputs[5], // Felt[1] - offered_asset_id_suffix
        inputs[6], // Felt[2] - padding (should be 0)
        inputs[7], // Felt[3] - offered_asset_total
    ]);

    // Step 3.5: Validate that offered_asset_word matches the asset in the active note
    // Get assets from the active note
    let note_assets = active_note::get_assets();

    // Check that there is exactly one asset in the note
    let num_assets = note_assets.len();
    assert_eq(Felt::from_u32(num_assets as u32), felt!(1));

    // Get the asset from the note
    let note_asset = note_assets[0];

    // Compare the offered asset from inputs with the asset in the note
    // They should match exactly (same asset ID and amount)
    let assets_match = if offered_asset_word == note_asset {
        felt!(1)
    } else {
        felt!(0)
    };
    assert_eq(assets_match, felt!(1));

    // Extract amounts for calculations
    let requested_asset_total = inputs[3];
    let offered_asset_total = inputs[7];

    // Note: note_serial_number is NOT in inputs - it's part of the note's structure
    // Get it using active_note::get_serial_number()
    let current_note_serial = active_note::get_serial_number();

    // Step 4: Validate input
    // If input_amount > requested_asset_total: FAIL
    let is_valid = if input_amount <= requested_asset_total {
        felt!(1)
    } else {
        felt!(0)
    };
    assert_eq(is_valid, felt!(1));

    // Step 5: Compute execution ratios
    // execution_ratio = input_amount / requested_asset_total
    // remainder_ratio = 1 - execution_ratio
    let one = felt!(1);
    // Step 6: Compute offered output
    // offered_out = offered_asset_total * execution_ratio
    let offered_out =
        calculate_output_amount(offered_asset_total, requested_asset_total, input_amount);

    active_note::add_assets_to_account();

    // Create routing (P2ID) note
    let routing_serial = add_word(
        current_note_serial,
        Word::from([felt!(0), felt!(0), felt!(0), felt!(1)]),
    );

    let aux_value = offered_out;
    let input_asset = Asset::new(Word::from([inputs[0], inputs[1], inputs[2], input_amount]));

    // Create P2ID note using output_note module
    create_p2id_note(
        routing_serial,
        input_asset,
        swapp_note_creator_id,
        aux_value,
    );

    // Create remainder swap note if partial fill
    if offered_out < offered_asset_total {
        let remainder_serial = hash_words(&[current_note_serial]).inner;
        let remainder_aux = offered_out;
        let remainder_requested_asset =
            Asset::from([inputs[0], inputs[1], inputs[2], inputs[3] - input_amount]);
        let remainder_offered_asset =
            Asset::from([inputs[4], inputs[5], inputs[6], inputs[7] - offered_out]);

        create_swapp_note(
            remainder_serial,
            remainder_requested_asset,
            remainder_offered_asset,
            swapp_note_creator_id,
            remainder_aux,
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
    if offered_total > requested_total {
        // Case 1: offered_total > requested_total
        // Calculate ratio = (offered_total * factor) / requested_total
        // Then output = (input_amount * ratio) / factor
        let ratio = (offered_total * precision_factor) / requested_total;
        return (input_amount * ratio) / precision_factor;
    } else {
        // Case 2: offered_total <= requested_total
        // Direct calculation: (input_amount * offered_total * factor) / (requested_total * factor)
        let ratio = (requested_total * precision_factor) / offered_total;
        return (input_amount * ratio) / precision_factor;
    }
}

/// Add two Words element-wise
fn add_word(a: Word, b: Word) -> Word {
    Word::from([a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]])
}

/// Create a P2ID (Pay-to-ID) note
fn create_p2id_note(serial_num: Word, input_asset: Asset, recipient_id: AccountId, aux: Felt) {
    // Create a tag for the P2ID note
    let tag = Tag::from(felt!(0));

    // Create a same note type as the active note
    let note_type = get_note_type();

    // Set execution hint (always executable for now)
    let execution_hint = felt!(0);

    let p2id_note_root_digest = Digest::from_word(Word::new([
        Felt::from_u64_unchecked(6412241294473976817),
        Felt::from_u64_unchecked(10671567784403105513),
        Felt::from_u64_unchecked(4275774806771663409),
        Felt::from_u64_unchecked(17933276983439992403),
    ]));

    // Create recipient from serial number and account ID
    // TODO: Create proper P2ID recipient with serial, script, and inputs
    // This is a placeholder - actual implementation needs proper P2ID recipient creation
    let recipient = Recipient::compute(
        serial_num,
        Digest::from_word(active_note::get_script_root()),
        vec![
            recipient_id.prefix,
            recipient_id.suffix,
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
        ],
    );

    // Create the note using output_note::create
    let note_idx = output_note::create(tag, aux, note_type, execution_hint, recipient);

    // Add the asset to the note
    output_note::add_asset(input_asset, note_idx);
}
/// Create a Swapp note with remainder parameters
fn create_swapp_note(
    serial_num: Word,
    offered_asset: Asset,
    requested_asset: Asset,
    note_creator_id: AccountId,
    aux: Felt,
) {
    // Create a same tag as the active note
    let tag = get_note_tag();

    // Create a same note type as the active note
    let note_type = get_note_type();

    // Set execution hint (always executable for now)
    let execution_hint = felt!(0);

    // Create recipient with swapp script and remainder parameters
    let recipient = Recipient::compute(
        serial_num,
        Digest::from_word(active_note::get_script_root()),
        vec![
            offered_asset.inner[0],
            offered_asset.inner[1],
            offered_asset.inner[2],
            offered_asset.inner[3],
            requested_asset.inner[0],
            requested_asset.inner[1],
            requested_asset.inner[2],
            requested_asset.inner[3],
            note_creator_id.prefix,
            note_creator_id.suffix,
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
            felt!(0),
        ],
    );

    // Create the note using output_note::create
    let note_idx = output_note::create(tag, aux, note_type, execution_hint, recipient);

    // Add the asset to the note
    output_note::add_asset(offered_asset, note_idx);
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
    let second_felt = metadata[1];
    // Shift left by 56 bits
    let left_shifted_56 = second_felt * Felt::from_u32(2u32.pow(56));
    // Shift right by 62
    let note_type_felt = left_shifted_56 / Felt::from_u32(2u32.pow(62));
    NoteType::from(note_type_felt)
}
