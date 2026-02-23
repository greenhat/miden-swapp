#![no_std]
#![feature(alloc_error_handler)]

#[macro_use]
extern crate alloc;

use crate::bindings::Account;
use alloc::vec;
use miden::*;

#[tx_script]
fn run(_arg: Word, _account: &mut Account) {
    // Load 4 words from the advice stack, verifying they match the commitment (RPO hash).
    // The caller pre-loads the advice stack and passes the hash as the script arg.
    let (_, data) = pipe_words_to_memory(felt!(4));
    // adv_load_preimage pops words from the top of the advice stack (LIFO)
    // and reverses elements within each word:
    //   Word [a, b, c, d] becomes [d, c, b, a] in the data buffer.
    //
    // Expected advice stack layout (top → bottom):
    //   Word 0 (data[0..3]):   serial_num
    //   Word 1 (data[4..7]):   [recipient_prefix, recipient_suffix, tag, note_type]
    //   Word 2 (data[8..11]):  [aux, 0, 0, 0]
    //   Word 3 (data[12..15]): asset_word

    // Serial num (Word 0 → data[0..3], reversed)
    let serial_num = Word::new([data[3], data[2], data[1], data[0]]);
    // Params (Word 1 → data[4..7], reversed: [note_type, tag, suffix, prefix])
    let note_type = NoteType::from(data[4]);
    let tag = Tag::from(data[5]);
    let recipient_suffix = data[6];
    let recipient_prefix = data[7];
    // Aux (Word 2 → data[8..11], reversed: [0, 0, 0, aux])
    let aux = data[11];
    // Asset word (Word 3 → data[12..15], reversed)
    let asset_word = Word::new([data[15], data[14], data[13], data[12]]);

    let recipient_id = AccountId::new(recipient_prefix, recipient_suffix);

    // P2ID note script root digest (same hardcoded value as in swapp-note)
    let p2id_note_root_digest = Digest::from_word(Word::new([
        Felt::from_u64_unchecked(13362761878458161062),
        Felt::from_u64_unchecked(15090726097241769395),
        Felt::from_u64_unchecked(444910447169617901),
        Felt::from_u64_unchecked(3558201871398422326),
    ]));

    // Compute recipient from serial number, P2ID script root, and account ID inputs
    let recipient = Recipient::compute(
        serial_num,
        p2id_note_root_digest,
        vec![recipient_id.suffix, recipient_id.prefix],
    );

    // Create the P2ID output note
    let note_idx = output_note::create(tag, note_type, recipient);

    // Set attachment with aux value
    output_note::set_word_attachment(
        note_idx,
        felt!(0),
        Word::from([aux, felt!(0), felt!(0), felt!(0)]),
    );

    // Add asset directly to the note (no account interaction)
    let asset = Asset::new(asset_word);
    output_note::add_asset(asset, note_idx);
}
