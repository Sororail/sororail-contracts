use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error, impl_single_position_storage};

use crate::types::Grant;

#[contracttype]
pub enum DataKey {
    /// The single grant held by this instance.
    Grant,
}

impl_single_position_storage!(Grant, DataKey::Grant);
