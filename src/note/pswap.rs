use alloc::vec::Vec;

use miden_crypto::utils::Deserializable;
use miden_mast_package::Package;
use miden_protocol::account::AccountId;
use miden_protocol::asset::Asset;
use miden_protocol::crypto::rand::FeltRng;
use miden_protocol::errors::NoteError;
use miden_protocol::note::{
    Note, NoteAssets, NoteAttachment, NoteInputs, NoteMetadata, NoteRecipient, NoteScript, NoteTag,
    NoteType,
};
use miden_protocol::utils::sync::LazyLock;
use miden_protocol::{Felt, Word, ZERO};

// NOTE SCRIPT
// ================================================================================================

// Initialize the SWAPP note script only once by loading the embedded package
static PSWAP_SCRIPT: LazyLock<NoteScript> = LazyLock::new(|| {
    // Read the compiled package directly from the contracts folder
    // Use CARGO_MANIFEST_DIR to get absolute path relative to workspace root
    let package_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/swapp-note/swapp_note.masp"
    );
    let package_bytes = std::fs::read(package_path).expect(&format!(
        "Failed to read compiled package from {}",
        package_path
    ));

    // Deserialize the package
    let package =
        Package::read_from_bytes(&package_bytes).expect("Failed to deserialize swapp-note package");

    // Extract the note script from the package
    let note_program = package.unwrap_program();
    NoteScript::from_parts(
        note_program.mast_forest().clone(),
        note_program.entrypoint(),
    )
});

// PSWAP NOTE
// ================================================================================================

/// Partial swap (pswap) note for decentralized asset exchange.
///
/// This note implements a partially-fillable swap mechanism where:
/// - Creator offers an asset and requests another asset
/// - Note can be partially or fully filled by consumers
/// - Unfilled portions create remainder notes
/// - Creator receives requested assets via P2ID notes
pub struct PswapNote;

impl PswapNote {
    // CONSTANTS
    // --------------------------------------------------------------------------------------------

    /// Expected number of input items for the PSWAP note.
    ///
    /// Layout (8 Felts):
    /// - [0-3]: Requested asset (faucet_id_prefix, faucet_id_suffix, padding, amount)
    /// - [4-5]: Creator account ID (prefix, suffix)
    /// - [6]: Note type
    /// - [7]: P2ID routing tag
    pub const NUM_INPUT_ITEMS: usize = 8;

    // PUBLIC ACCESSORS
    // --------------------------------------------------------------------------------------------

    /// Returns the script of the PSWAP note.
    pub fn script() -> NoteScript {
        PSWAP_SCRIPT.clone()
    }

    /// Returns the PSWAP note script root.
    pub fn script_root() -> Word {
        PSWAP_SCRIPT.root()
    }

    // BUILDERS
    // --------------------------------------------------------------------------------------------

    /// Creates a PSWAP note offering one asset in exchange for another.
    ///
    /// # Arguments
    ///
    /// * `creator_account_id` - The account creating the swap offer
    /// * `offered_asset` - The asset being offered (will be locked in the note)
    /// * `requested_asset` - The asset being requested in exchange
    /// * `note_type` - Whether the note is public or private
    /// * `note_attachment` - Optional attachment data
    /// * `rng` - Random number generator for serial number
    ///
    /// # Returns
    ///
    /// Returns a `Note` that can be consumed by anyone willing to provide the requested asset.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Assets are invalid or have the same faucet ID
    /// - Note construction fails
    pub fn create<R: FeltRng>(
        creator_account_id: AccountId,
        offered_asset: Asset,
        requested_asset: Asset,
        note_type: NoteType,
        note_attachment: NoteAttachment,
        rng: &mut R,
    ) -> Result<Note, NoteError> {
        // Validate that offered and requested assets are different
        if offered_asset.faucet_id_prefix() == requested_asset.faucet_id_prefix() {
            return Err(NoteError::other(
                "Offered and requested assets must be different",
            ));
        }

        let note_script = Self::script();

        let (faucet_prefix, faucet_suffix, amount) = match &requested_asset {
            Asset::Fungible(fa) => (
                fa.faucet_id().prefix().as_felt(),
                fa.faucet_id().suffix(),
                Felt::new(fa.amount()),
            ),
            Asset::NonFungible(_nfa) => {
                return Err(NoteError::other("Non-fungible assets not yet supported"));
            }
        };

        // Build note inputs (8 Felts)
        let p2id_tag_felt = Self::compute_p2id_tag_felt(creator_account_id);

        let inputs = vec![
            faucet_prefix, // requested_asset.faucet_id().prefix()
            faucet_suffix, // requested_asset.faucet_id().suffix()
            Felt::new(0),  // padding
            amount,        // requested_asset.amount()
            creator_account_id.prefix().as_felt(),
            creator_account_id.suffix(),
            note_type.into(),
            p2id_tag_felt,
        ];

        let note_inputs = NoteInputs::new(inputs)?;

        // Build the tag for the PSWAP use case
        let tag = Self::build_tag(note_type, &offered_asset, &requested_asset);

        // Generate serial number
        let serial_num = rng.draw_word();

        // Build the outgoing note
        let metadata =
            NoteMetadata::new(creator_account_id, note_type, tag).with_attachment(note_attachment);

        let assets = NoteAssets::new(vec![offered_asset])?;
        let recipient = NoteRecipient::new(serial_num, note_script, note_inputs);
        let note = Note::new(assets, metadata, recipient);

        Ok(note)
    }

    // TAG CONSTRUCTION
    // --------------------------------------------------------------------------------------------

    /// Returns a note tag for a pswap note with the specified parameters.
    ///
    /// The tag is laid out as follows:
    ///
    /// ```text
    /// [
    ///   note_type (2 bits) | script_root (14 bits)
    ///   | offered_asset_faucet_id (8 bits) | requested_asset_faucet_id (8 bits)
    /// ]
    /// ```
    ///
    /// The script root serves as the use case identifier of the PSWAP tag.
    pub fn build_tag(
        note_type: NoteType,
        offered_asset: &Asset,
        requested_asset: &Asset,
    ) -> NoteTag {
        let pswap_root_bytes = Self::script().root().as_bytes();

        // Construct the pswap use case ID from the 14 most significant bits of the script root
        // This leaves the two most significant bits zero
        let mut pswap_use_case_id = (pswap_root_bytes[0] as u16) << 6;
        pswap_use_case_id |= (pswap_root_bytes[1] >> 2) as u16;

        // Get bits 0..8 from the faucet IDs of both assets which will form the tag payload
        let offered_asset_id: u64 = offered_asset.faucet_id_prefix().into();
        let offered_asset_tag = (offered_asset_id >> 56) as u8;

        let requested_asset_id: u64 = requested_asset.faucet_id_prefix().into();
        let requested_asset_tag = (requested_asset_id >> 56) as u8;

        let asset_pair = ((offered_asset_tag as u16) << 8) | (requested_asset_tag as u16);

        let tag = ((note_type as u8 as u32) << 30)
            | ((pswap_use_case_id as u32) << 16)
            | asset_pair as u32;

        NoteTag::new(tag)
    }

    // HELPER FUNCTIONS
    // --------------------------------------------------------------------------------------------

    /// Computes the P2ID tag for routing payback notes to the creator.
    fn compute_p2id_tag_felt(account_id: AccountId) -> Felt {
        let p2id_tag = NoteTag::with_account_target(account_id);
        Felt::new(u32::from(p2id_tag) as u64)
    }

    // PARSING FUNCTIONS
    // --------------------------------------------------------------------------------------------

    /// Parses note inputs to extract swap parameters.
    ///
    /// # Arguments
    ///
    /// * `inputs` - The note inputs (must be exactly 8 Felts)
    ///
    /// # Returns
    ///
    /// Returns a tuple containing:
    /// - `requested_asset_word`: The requested asset as a Word
    /// - `creator_account_id`: The account ID of the swap creator
    /// - `note_type`: The note type for payback notes
    /// - `p2id_tag`: The tag for routing payback notes
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Input length is not 8
    /// - Account ID construction fails
    pub fn parse_inputs(
        inputs: &[Felt],
    ) -> Result<(Word, AccountId, NoteType, NoteTag), NoteError> {
        if inputs.len() != Self::NUM_INPUT_ITEMS {
            return Err(NoteError::other(alloc::format!(
                "PSWAP note should have {} inputs, but {} were provided",
                Self::NUM_INPUT_ITEMS,
                inputs.len()
            )));
        }

        // Extract requested asset word
        let requested_asset_word = Word::from([
            inputs[0], // faucet_id_prefix
            inputs[1], // faucet_id_suffix
            inputs[2], // padding (should be 0)
            inputs[3], // amount
        ]);

        // Extract creator account ID
        let creator_prefix = inputs[4];
        let creator_suffix = inputs[5];
        let creator_account_id =
            AccountId::try_from([creator_suffix, creator_prefix]).map_err(|e| {
                NoteError::other(alloc::format!("Failed to parse creator account ID: {}", e))
            })?;

        // Extract note type and tag
        let note_type = NoteType::try_from(inputs[6].as_int() as u8)
            .map_err(|e| NoteError::other(alloc::format!("Failed to parse note type: {}", e)))?;

        let p2id_tag = NoteTag::new(inputs[7].as_int() as u32);

        Ok((
            requested_asset_word,
            creator_account_id,
            note_type,
            p2id_tag,
        ))
    }

    /// Extracts the requested asset from note inputs.
    ///
    /// # Arguments
    ///
    /// * `inputs` - The note inputs
    ///
    /// # Returns
    ///
    /// Returns the requested `Asset`.
    pub fn get_requested_asset(inputs: &[Felt]) -> Result<Asset, NoteError> {
        let (requested_asset_word, _, _, _) = Self::parse_inputs(inputs)?;
        Asset::try_from(requested_asset_word)
            .map_err(|e| NoteError::other(alloc::format!("Failed to parse asset from word: {}", e)))
    }

    /// Extracts the creator account ID from note inputs.
    ///
    /// # Arguments
    ///
    /// * `inputs` - The note inputs
    ///
    /// # Returns
    ///
    /// Returns the creator's `AccountId`.
    pub fn get_creator_account_id(inputs: &[Felt]) -> Result<AccountId, NoteError> {
        let (_, creator_account_id, _, _) = Self::parse_inputs(inputs)?;
        Ok(creator_account_id)
    }

    /// Checks if the given account is the creator of this swap note.
    ///
    /// # Arguments
    ///
    /// * `inputs` - The note inputs
    /// * `account_id` - The account ID to check
    ///
    /// # Returns
    ///
    /// Returns `true` if the account is the creator, `false` otherwise.
    pub fn is_creator(inputs: &[Felt], account_id: AccountId) -> Result<bool, NoteError> {
        let creator_id = Self::get_creator_account_id(inputs)?;
        Ok(creator_id == account_id)
    }

    /// Calculates the output amount for a partial fill.
    ///
    /// This uses the same proportional calculation as the on-chain script.
    ///
    /// # Arguments
    ///
    /// * `offered_total` - Total offered asset amount in the note
    /// * `requested_total` - Total requested asset amount
    /// * `input_amount` - Amount of requested asset being provided
    ///
    /// # Returns
    ///
    /// Returns the proportional amount of offered asset to receive.
    pub fn calculate_output_amount(
        offered_total: u64,
        requested_total: u64,
        input_amount: u64,
    ) -> u64 {
        const PRECISION_FACTOR: u64 = 100_000;

        if offered_total > requested_total {
            // Case 1: offered_total > requested_total
            // Calculate ratio = (offered_total * factor) / requested_total
            // Then output = (input_amount * ratio) / factor
            let ratio = (offered_total * PRECISION_FACTOR) / requested_total;
            (input_amount * ratio) / PRECISION_FACTOR
        } else {
            // Case 2: offered_total <= requested_total
            // Direct calculation with precision
            let ratio = (requested_total * PRECISION_FACTOR) / offered_total;
            (input_amount * PRECISION_FACTOR) / ratio
        }
    }
}

// TESTS
// ================================================================================================

#[cfg(test)]
mod tests {
    use miden_protocol::account::{AccountId, AccountIdVersion, AccountStorageMode, AccountType};
    use miden_protocol::asset::FungibleAsset;

    use super::*;

    #[test]
    fn test_pswap_note_creation_and_script() {
        // Test that the LazyLock PSWAP_SCRIPT initializes correctly and note creation works

        // Create test faucet IDs
        let mut offered_faucet_bytes = [0; 15];
        offered_faucet_bytes[0] = 0xaa;

        let mut requested_faucet_bytes = [0; 15];
        requested_faucet_bytes[0] = 0xbb;

        let offered_faucet_id = AccountId::dummy(
            offered_faucet_bytes,
            AccountIdVersion::Version0,
            AccountType::FungibleFaucet,
            AccountStorageMode::Public,
        );

        let requested_faucet_id = AccountId::dummy(
            requested_faucet_bytes,
            AccountIdVersion::Version0,
            AccountType::FungibleFaucet,
            AccountStorageMode::Public,
        );

        // Create creator account
        let creator_id = AccountId::dummy(
            [1; 15],
            AccountIdVersion::Version0,
            AccountType::RegularAccountImmutableCode,
            AccountStorageMode::Public,
        );

        // Create assets
        let offered_asset = Asset::Fungible(FungibleAsset::new(offered_faucet_id, 1000).unwrap());
        let requested_asset =
            Asset::Fungible(FungibleAsset::new(requested_faucet_id, 500).unwrap());

        // Create RNG
        use miden_crypto::rand::RpoRandomCoin;
        let mut rng = RpoRandomCoin::new(Word::default());

        // Test that the script can be accessed (this will trigger LazyLock initialization)
        let script = PswapNote::script();
        assert!(
            script.root() != Word::default(),
            "Script root should not be zero"
        );
        println!(
            "✅ Script loaded successfully with root: {:?}",
            script.root()
        );

        // Create a PSWAP note
        let note = PswapNote::create(
            creator_id,
            offered_asset,
            requested_asset,
            NoteType::Public,
            NoteAttachment::default(),
            &mut rng,
        );

        assert!(note.is_ok(), "Note creation should succeed");
        let note = note.unwrap();

        // Verify note properties
        assert_eq!(
            note.metadata().sender(),
            creator_id,
            "Note sender should match creator"
        );
        assert_eq!(
            note.metadata().note_type(),
            NoteType::Public,
            "Note type should be Public"
        );
        assert_eq!(note.assets().num_assets(), 1, "Note should have 1 asset");

        // Verify the note has the correct script
        assert_eq!(
            note.recipient().script().root(),
            script.root(),
            "Note script should match PSWAP script"
        );

        println!(
            "✅ PSWAP note created successfully with ID: {:?}",
            note.id()
        );
    }

    #[test]
    fn test_pswap_tag() {
        // Construct test faucet IDs
        let mut offered_faucet_bytes = [0; 15];
        offered_faucet_bytes[0] = 0xcd;
        offered_faucet_bytes[1] = 0xb1;

        let mut requested_faucet_bytes = [0; 15];
        requested_faucet_bytes[0] = 0xab;
        requested_faucet_bytes[1] = 0xec;

        let offered_asset = Asset::Fungible(
            FungibleAsset::new(
                AccountId::dummy(
                    offered_faucet_bytes,
                    AccountIdVersion::Version0,
                    AccountType::FungibleFaucet,
                    AccountStorageMode::Public,
                ),
                2500,
            )
            .unwrap(),
        );

        let requested_asset = Asset::Fungible(
            FungibleAsset::new(
                AccountId::dummy(
                    requested_faucet_bytes,
                    AccountIdVersion::Version0,
                    AccountType::FungibleFaucet,
                    AccountStorageMode::Public,
                ),
                5000,
            )
            .unwrap(),
        );

        let expected_asset_pair = 0xcdab;

        let note_type = NoteType::Public;
        let actual_tag = PswapNote::build_tag(note_type, &offered_asset, &requested_asset);

        assert_eq!(
            actual_tag.as_u32() as u16,
            expected_asset_pair,
            "asset pair should match"
        );
        assert_eq!(
            (actual_tag.as_u32() >> 30) as u8,
            note_type as u8,
            "note type should match"
        );
    }

    #[test]
    fn test_calculate_output_amount() {
        // Test 1: offered > requested (e.g., 1000 USDT offered for 500 ETH)
        let output = PswapNote::calculate_output_amount(1000, 500, 250);
        assert_eq!(output, 500); // Should get 500 USDT for 250 ETH

        // Test 2: offered < requested (e.g., 500 ETH offered for 1000 USDT)
        let output = PswapNote::calculate_output_amount(500, 1000, 500);
        assert_eq!(output, 250); // Should get 250 ETH for 500 USDT

        // Test 3: offered == requested (1:1 ratio)
        let output = PswapNote::calculate_output_amount(1000, 1000, 500);
        assert_eq!(output, 500); // Should get 500 for 500

        // Test 4: Partial fill
        let output = PswapNote::calculate_output_amount(10000, 5000, 1000);
        assert_eq!(output, 2000); // Should get 2000 for 1000
    }

    #[test]
    fn test_parse_inputs() {
        // Create test inputs
        let faucet_id = AccountId::dummy(
            [0; 15],
            AccountIdVersion::Version0,
            AccountType::FungibleFaucet,
            AccountStorageMode::Public,
        );

        let creator_id = AccountId::dummy(
            [1; 15],
            AccountIdVersion::Version0,
            AccountType::RegularAccountImmutableCode,
            AccountStorageMode::Public,
        );

        let requested_asset = Asset::Fungible(FungibleAsset::new(faucet_id, 5000).unwrap());

        let requested_asset_word: Word = requested_asset.into();

        let note_type = NoteType::Public;
        let p2id_tag = NoteTag::with_account_target(creator_id);
        let p2id_tag_felt = Felt::new(u32::from(p2id_tag) as u64);

        let inputs = vec![
            requested_asset_word[0],
            requested_asset_word[1],
            ZERO,
            requested_asset_word[3],
            creator_id.prefix().as_felt(),
            creator_id.suffix(),
            note_type.into(),
            p2id_tag_felt,
        ];

        // Parse and verify
        let (parsed_asset_word, parsed_creator, parsed_note_type, parsed_tag) =
            PswapNote::parse_inputs(&inputs).unwrap();

        assert_eq!(parsed_asset_word, requested_asset_word);
        assert_eq!(parsed_creator, creator_id);
        assert_eq!(parsed_note_type, note_type);
        assert_eq!(parsed_tag, p2id_tag);
    }
}
