use miden_crypto::utils::Deserializable;
use miden_mast_package::Package;
use miden_protocol::account::component::InitStorageData;
use miden_protocol::account::{Account, AccountBuilder, AccountComponent, AccountStorageMode, AccountType};
use miden_protocol::asset::Asset;
use miden_protocol::utils::sync::LazyLock;

use alloc::collections::BTreeSet;

// ACCOUNT COMPONENT
// ================================================================================================

/// Initialize the basic-wallet account component only once by loading the embedded package.
static BASIC_WALLET_COMPONENT: LazyLock<AccountComponent> = LazyLock::new(|| {
    let package_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/basic-wallet/basic_wallet.masp"
    );
    let package_bytes = std::fs::read(package_path).expect(&format!(
        "Failed to read compiled package from {}",
        package_path
    ));

    let package =
        Package::read_from_bytes(&package_bytes).expect("Failed to deserialize basic-wallet package");

    let init_storage_data = InitStorageData::default();

    AccountComponent::from_package(&package, &init_storage_data)
        .expect("Failed to create account component from basic-wallet package")
        .with_supported_types(BTreeSet::from_iter([AccountType::RegularAccountImmutableCode]))
});

// BASIC WALLET
// ================================================================================================

/// SDK-side representation of the basic-wallet account component.
///
/// This mirrors `contracts/basic-wallet/` and provides helpers for loading
/// the compiled component and building accounts that use it.
pub struct BasicWallet;

impl BasicWallet {
    /// Returns the loaded basic-wallet account component.
    pub fn component() -> AccountComponent {
        BASIC_WALLET_COMPONENT.clone()
    }

    /// Creates an account with the basic-wallet component and the given initial assets.
    ///
    /// # Arguments
    ///
    /// * `account_id` - Seed bytes for the account builder
    /// * `assets` - Initial assets to fund the account with
    /// * `storage_mode` - Account storage mode (e.g. `Public`)
    pub fn create(
        init_seed: [u8; 32],
        assets: Vec<Asset>,
        storage_mode: AccountStorageMode,
    ) -> Account {
        AccountBuilder::new(init_seed)
            .account_type(AccountType::RegularAccountImmutableCode)
            .storage_mode(storage_mode)
            .with_component(Self::component())
            .with_assets(assets)
            .build_existing()
            .expect("Failed to build basic-wallet account")
    }
}

// TESTS
// ================================================================================================

#[cfg(test)]
mod tests {
    use miden_protocol::account::{AccountIdVersion, AccountStorageMode, AccountType};
    use miden_protocol::account::AccountId;
    use miden_protocol::asset::{Asset, FungibleAsset};

    use super::*;

    /// Helper to create a dummy faucet ID for testing.
    fn dummy_faucet(byte: u8) -> AccountId {
        let mut bytes = [0u8; 15];
        bytes[0] = byte;
        AccountId::dummy(
            bytes,
            AccountIdVersion::Version0,
            AccountType::FungibleFaucet,
            AccountStorageMode::Public,
        )
    }

    #[test]
    fn test_component_loads_successfully() {
        // Trigger LazyLock initialization and verify the component is valid.
        let component = BasicWallet::component();

        // The component should have supported types set.
        let supported = component.supported_account_types();
        assert!(
            supported.contains(&AccountType::RegularAccountImmutableCode),
            "Component should support RegularAccountImmutableCode"
        );
    }

    #[test]
    fn test_component_is_idempotent() {
        // Calling component() multiple times should return the same component (LazyLock).
        let c1 = BasicWallet::component();
        let c2 = BasicWallet::component();

        assert_eq!(
            c1.supported_account_types(),
            c2.supported_account_types(),
            "Multiple calls should return equivalent components"
        );
    }

    #[test]
    fn test_create_account_no_assets() {
        let account = BasicWallet::create(
            [1u8; 32],
            vec![],
            AccountStorageMode::Public,
        );

        assert!(
            account.vault().assets().count() == 0,
            "Account created with no assets should have an empty vault"
        );
    }

    #[test]
    fn test_create_account_with_single_asset() {
        let faucet = dummy_faucet(0xaa);
        let asset = Asset::Fungible(FungibleAsset::new(faucet, 500).unwrap());

        let account = BasicWallet::create(
            [2u8; 32],
            vec![asset],
            AccountStorageMode::Public,
        );

        let vault_assets: Vec<Asset> = account.vault().assets().collect();
        assert_eq!(vault_assets.len(), 1, "Account should have 1 asset");
        match &vault_assets[0] {
            Asset::Fungible(fa) => {
                assert_eq!(fa.faucet_id(), faucet);
                assert_eq!(fa.amount(), 500);
            }
            _ => panic!("Expected fungible asset"),
        }
    }

    #[test]
    fn test_create_account_with_multiple_assets() {
        let faucet_a = dummy_faucet(0xaa);
        let faucet_b = dummy_faucet(0xbb);

        let assets = vec![
            Asset::Fungible(FungibleAsset::new(faucet_a, 100).unwrap()),
            Asset::Fungible(FungibleAsset::new(faucet_b, 200).unwrap()),
        ];

        let account = BasicWallet::create(
            [3u8; 32],
            assets,
            AccountStorageMode::Public,
        );

        let vault_assets: Vec<Asset> = account.vault().assets().collect();
        assert_eq!(vault_assets.len(), 2, "Account should have 2 assets");
    }

    #[test]
    fn test_different_seeds_produce_different_accounts() {
        let account_a = BasicWallet::create([10u8; 32], vec![], AccountStorageMode::Public);
        let account_b = BasicWallet::create([20u8; 32], vec![], AccountStorageMode::Public);

        assert_ne!(
            account_a.id(),
            account_b.id(),
            "Different seeds should produce different account IDs"
        );
    }
}
