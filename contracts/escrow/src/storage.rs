use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error, impl_single_position_storage};

use crate::types::Escrow;

#[contracttype]
pub enum DataKey {
    /// The single escrow agreement held by this instance.
    Escrow,
}

impl_single_position_storage!(Escrow, DataKey::Escrow);
