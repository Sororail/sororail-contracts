use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error, impl_single_position_storage};

use crate::types::Stream;

#[contracttype]
pub enum DataKey {
    /// The single stream held by this instance.
    Stream,
}

impl_single_position_storage!(Stream, DataKey::Stream);
