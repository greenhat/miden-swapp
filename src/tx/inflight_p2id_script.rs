use alloc::vec;
use alloc::vec::Vec;

use miden_core::crypto::hash::Rpo256;
use miden_crypto::utils::Deserializable;
use miden_mast_package::Package;
use miden_protocol::account::AccountId;
use miden_protocol::asset::Asset;
use miden_protocol::crypto::rand::FeltRng;
use miden_protocol::note::{
    Note, NoteAssets, NoteAttachment, NoteAttachmentScheme, NoteMetadata, NoteTag, NoteType,
};
use miden_protocol::transaction::TransactionScript;
use miden_protocol::utils::sync::LazyLock;
use miden_protocol::{Felt, Word, ZERO};
use miden_standards::note::utils::build_p2id_recipient;

// TX SCRIPT
// ================================================================================================

const P2ID_TX_SCRIPT_BYTES: &[u8] =
    include_bytes!("../../contracts/p2id-tx-script/p2id_tx_script.masp");

/// Load the compiled p2id-tx-script program once.
static P2ID_TX_SCRIPT_PROGRAM: LazyLock<TransactionScript> = LazyLock::new(|| {
    let package = Package::read_from_bytes(P2ID_TX_SCRIPT_BYTES)
        .expect("Failed to deserialize p2id-tx-script package");
    let program = package.unwrap_program();
    TransactionScript::from_parts(program.mast_forest().clone(), program.entrypoint())
});

// INFLIGHT P2ID SCRIPT
// ================================================================================================

/// Output of [`InflightP2idScript::prepare`]: everything needed to wire
/// the p2id-tx-script into a transaction context.
pub struct InflightP2idData {
    /// The commitment word passed as the tx-script argument.
    pub commitment_arg: Word,
    /// `(key, value)` pair to feed into `extend_advice_map`.
    pub advice_map_entry: (Word, Vec<Felt>),
    /// Expected output notes (one per recipient).
    pub expected_notes: Vec<Note>,
}

/// SDK helper for the **p2id-tx-script**: creates N inflight P2ID output
/// notes in a single transaction.
///
/// # Usage
///
/// ```ignore
/// use miden_swapp::InflightP2idScript;
///
/// // 1. Get the TransactionScript
/// let tx_script = InflightP2idScript::tx_script();
///
/// // 2. Prepare advice data
/// let data = InflightP2idScript::prepare(
///     sender_id,
///     &[(asset1, recipient1), (asset2, recipient2)],
///     &mut rng,
/// )?;
///
/// // 3. Wire into the transaction context
/// let tx_context = mock_chain
///     .build_tx_context(sender_id, &input_note_ids, &[])?
///     .tx_script(tx_script)
///     .tx_script_args(data.commitment_arg)
///     .extend_advice_map([data.advice_map_entry])
///     .extend_expected_output_notes(
///         data.expected_notes.into_iter().map(OutputNote::Full).collect(),
///     )
///     .build()?;
/// ```
pub struct InflightP2idScript;

impl InflightP2idScript {
    /// Returns the compiled `TransactionScript` for the p2id-tx-script.
    pub fn tx_script() -> TransactionScript {
        P2ID_TX_SCRIPT_PROGRAM.clone()
    }

    /// Prepares everything needed to create N inflight P2ID notes.
    ///
    /// # Arguments
    ///
    /// * `sender_id` – the account executing the transaction (note metadata sender).
    /// * `notes` – slice of `(asset, recipient_id)` pairs, one per output note.
    /// * `rng` – random source used to generate unique serial numbers.
    ///
    /// # Returns
    ///
    /// An [`InflightP2idData`] containing the commitment arg, advice map entry,
    /// and the expected output `Note` objects.
    pub fn prepare<R: FeltRng>(
        sender_id: AccountId,
        notes: &[(Asset, AccountId)],
        rng: &mut R,
    ) -> Result<InflightP2idData, miden_protocol::errors::NoteError> {
        assert!(!notes.is_empty(), "must provide at least one note");

        let note_type = NoteType::Public;
        let note_type_felt: Felt = note_type.into();

        let mut advice_felts: Vec<Felt> = Vec::with_capacity(notes.len() * 16);
        let mut expected_notes: Vec<Note> = Vec::with_capacity(notes.len());

        for (asset, recipient_id) in notes {
            let serial_num: Word = rng.draw_word();
            let tag = NoteTag::with_account_target(*recipient_id);
            let tag_felt = Felt::new(u32::from(tag) as u64);
            let asset_word = Word::from(*asset);

            let amount = match asset {
                Asset::Fungible(fa) => Felt::new(fa.amount()),
                Asset::NonFungible(_) => ZERO,
            };

            // 16 felts per note (4 words)
            // Word 0: serial_num
            advice_felts.extend(serial_num);
            // Word 1: [recipient_prefix, recipient_suffix, tag, note_type]
            advice_felts.push(recipient_id.prefix().into());
            advice_felts.push(recipient_id.suffix());
            advice_felts.push(tag_felt);
            advice_felts.push(note_type_felt);
            // Word 2: [aux, 0, 0, 0]
            advice_felts.push(amount);
            advice_felts.push(ZERO);
            advice_felts.push(ZERO);
            advice_felts.push(ZERO);
            // Word 3: asset_word
            advice_felts.extend(asset_word);

            // Build the expected output note
            let recipient = build_p2id_recipient(*recipient_id, serial_num)?;
            let note_assets = NoteAssets::new(vec![*asset])?;
            let aux_word = Word::from([amount, ZERO, ZERO, ZERO]);
            let attachment = NoteAttachment::new_word(NoteAttachmentScheme::none(), aux_word);
            let metadata =
                NoteMetadata::new(sender_id, note_type, tag).with_attachment(attachment);
            expected_notes.push(Note::new(note_assets, metadata, recipient));
        }

        // Compute RPO commitment over all advice felts
        let commitment_key: Word = Rpo256::hash_elements(&advice_felts);
        let mut commitment_arg = commitment_key;
        commitment_arg.reverse();

        Ok(InflightP2idData {
            commitment_arg,
            advice_map_entry: (commitment_key, advice_felts),
            expected_notes,
        })
    }
}

// TESTS
// ================================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_script_loads() {
        let _script = InflightP2idScript::tx_script();
    }
}
