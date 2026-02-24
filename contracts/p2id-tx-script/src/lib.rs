#![no_std]
#![feature(alloc_error_handler)]

#[macro_use]
extern crate alloc;

use crate::bindings::Account;
use alloc::vec;
use miden::{intrinsics::advice::adv_push_mapvaln, *};

#[tx_script]
fn run(arg: Word, _account: &mut Account) {
    let num_felts = adv_push_mapvaln(arg.clone());
    let num_felts_u64 = num_felts.as_u64();

    // Each note = 4 words = 16 felts; total must be a multiple of 16
    assert_eq(Felt::from_u32((num_felts_u64 % 16) as u32), felt!(0));
    let num_notes = num_felts_u64 / 16;
    let num_words = Felt::from_u64_unchecked(num_felts_u64 / 4);

    // Load all words at once, verified against the commitment (RPO hash)
    let data = adv_load_preimage(num_words, arg);

    // P2ID note script root digest (same hardcoded value as in swapp-note)
    let p2id_note_root_digest = Digest::from_word(Word::new([
        Felt::from_u64_unchecked(13362761878458161062),
        Felt::from_u64_unchecked(15090726097241769395),
        Felt::from_u64_unchecked(444910447169617901),
        Felt::from_u64_unchecked(3558201871398422326),
    ]));

    // Create one P2ID note per 4-word block
    for i in 0..num_notes {
        let off = (i * 16) as usize;

        // Word 0: serial_num
        let serial_num = Word::new([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        // Word 1: [recipient_prefix, recipient_suffix, tag, note_type]
        let recipient_prefix = data[off + 4];
        let recipient_suffix = data[off + 5];
        let tag = Tag::from(data[off + 6]);
        let note_type = NoteType::from(data[off + 7]);
        // Word 2: [aux, 0, 0, 0]
        let aux = data[off + 8];
        // Word 3: asset_word
        let asset_word = Word::new([
            data[off + 12],
            data[off + 13],
            data[off + 14],
            data[off + 15],
        ]);

        let recipient_id = AccountId::new(recipient_prefix, recipient_suffix);

        // Compute recipient from serial number, P2ID script root, and account ID inputs
        let recipient = Recipient::compute(
            serial_num,
            p2id_note_root_digest.clone(),
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
        output_note::add_asset(Asset::new(asset_word), note_idx);
    }
}
