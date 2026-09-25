use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error, impl_single_position_storage};

use crate::types::Authorization;

#[contracttype]
pub enum DataKey {
    /// The single authorization held by this instance.
    Auth,
}

impl_single_position_storage!(Authorization, DataKey::Auth);
