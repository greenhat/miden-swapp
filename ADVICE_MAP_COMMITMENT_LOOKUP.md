# Advice Map Commitment Lookup Locations

This document points to the key locations where the Miden VM checks the advice map using commitments.

## Key Locations

### 1. **Note Inputs Lookup** (Primary Location)
**File**: `/Users/vaibhavjindal/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/miden-lib-0.12.4/asm/miden/active_note.masm`

**Procedure**: `write_inputs_to_memory` (lines 333-374)
- **Line 335**: `adv.push_mapvaln` - This is where the advice map is queried using `NOTE_INPUTS_COMMITMENT` as the key
- **Input**: `[NOTE_INPUTS_COMMITMENT, num_inputs, dest_ptr]`
- **Advice Map Key**: `NOTE_INPUTS_COMMITMENT` (the commitment hash of the note inputs)
- **Retrieves**: The actual input values from the advice map

```masm
proc.write_inputs_to_memory
    # load the inputs from the advice map to the advice stack
    adv.push_mapvaln  # <-- LINE 335: Advice map lookup using commitment
    # OS => [NOTE_INPUTS_COMMITMENT, num_inputs, dest_ptr]
    # AS => [advice_num_inputs, [INPUT_VALUES]]
```

**Called from**: `export.get_inputs` (line 106)
- This procedure gets the `NOTE_INPUTS_COMMITMENT` from the kernel
- Then calls `write_inputs_to_memory` which performs the advice map lookup

### 2. **Note Assets Lookup**
**File**: `/Users/vaibhavjindal/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/miden-lib-0.12.4/asm/miden/note.masm`

**Procedure**: `export.write_assets_to_memory` (lines 68-88)
- **Line 70**: `adv.push_mapval` - Looks up assets using `ASSETS_COMMITMENT` as the key
- **Input**: `[ASSETS_COMMITMENT, num_assets, dest_ptr]`
- **Advice Map Key**: `ASSETS_COMMITMENT` (the sequential hash of the note's assets)

```masm
export.write_assets_to_memory
    # load the asset data from the advice map to the advice stack
    adv.push_mapval  # <-- LINE 70: Advice map lookup for assets
    # OS => [ASSETS_COMMITMENT, num_assets, dest_ptr]
    # AS => [[ASSETS_DATA]]
```

### 3. **Input Notes Data Loading** (Transaction Prologue)
**File**: `/Users/vaibhavjindal/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/miden-lib-0.12.4/asm/kernels/transaction/lib/prologue.masm`

**Procedure**: Input notes processing (lines 997-1003)
- **Line 1001**: `adv.push_mapval` - Loads input notes data from advice map
- Uses `INPUT_NOTES_COMMITMENT` as the key to retrieve all input notes data

```masm
# if there are input notes, load input notes data from the advice map onto the advice stack
dup neq.0
if.true
    exec.memory::get_input_notes_commitment
    adv.push_mapval  # <-- LINE 1001: Advice map lookup for input notes
    dropw
end
```

### 4. **How Commitments Are Computed**

#### For Input Notes:
**File**: `/Users/vaibhavjindal/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/miden-lib-0.12.4/asm/kernels/transaction/lib/prologue.masm`

**Procedure**: `compute_input_note_id` (lines 821-841)
- Computes `RECIPIENT` using `INPUT_COMMITMENT` (line 831)
- Computes `NOTE_ID` using `RECIPIENT` and `ASSETS_COMMITMENT` (line 839)

The inputs commitment is computed as a sequential hash of the padded note inputs.

#### For Output Notes:
**File**: `/Users/vaibhavjindal/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/miden-lib-0.12.4/asm/kernels/transaction/lib/output_note.masm`

**Procedure**: `get_assets_info` (lines 135-175)
- **Line 145**: Computes `ASSETS_COMMITMENT` for output notes
- **Line 169**: Stores assets to advice map using `adv.insert_mem` with the commitment as key

### 5. **In Your Test Code**
**File**: `/Users/vaibhavjindal/miden-swapp/integration/tests/swapp_test.rs`

**Lines 138-153**: You compute a commitment and store it in the advice map
```rust
// Build the input vector for the advice map (must be word-aligned)
let mut input: Vec<Felt> = vec![tag.into(), aux, note_type.into(), execution_hint.into()];
// Add recipient digest (4 Felts)
let recipient_digest: [Felt; 4] = recipient.digest().into();
input.extend(recipient_digest);
// Add the asset (4 Felts = 1 Word)
let asset = FungibleAsset::new(eth_faucet.id(), 25)?;
let asset_word: Word = asset.into();
input.extend(asset_word);

// Hash the input to create the commitment (this is the lookup key)
let commitment: [Felt; 4] = miden_core::crypto::hash::Rpo256::hash_elements(&input).into();

// Create the advice map: commitment -> input data
let mut advice_map = BTreeMap::new();
advice_map.insert(commitment.into(), input.clone());
```

However, **this commitment format doesn't match the standard Miden note structure**. The standard structure uses:
- `NOTE_INPUTS_COMMITMENT` for inputs
- `ASSETS_COMMITMENT` for assets

The commitment you're computing appears to be for a custom note structure that includes metadata (tag, aux, note_type, execution_hint, recipient, asset) in a single hash.

## Summary

The primary location where the advice map is checked using a commitment is:

1. **`write_inputs_to_memory`** in `active_note.masm` (line 335) - Uses `adv.push_mapvaln` with `NOTE_INPUTS_COMMITMENT`
2. **`write_assets_to_memory`** in `note.masm` (line 70) - Uses `adv.push_mapval` with `ASSETS_COMMITMENT`
3. **Transaction prologue** in `prologue.masm` (line 1001) - Uses `adv.push_mapval` with `INPUT_NOTES_COMMITMENT`

These procedures compute or receive a commitment hash, then use it as a key to retrieve the corresponding data from the advice map.



