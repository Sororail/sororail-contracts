use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error};

use crate::types::Escrow;

#[contracttype]
pub enum DataKey {
    /// The single escrow agreement held by this instance.
    Escrow,
}

/// Whether `init` has run.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Escrow)
}

/// Loads the agreement for a mutating operation and extends the instance TTL.
pub fn load(env: &Env) -> Result<Escrow, Error> {
    let escrow = env
        .storage()
        .instance()
        .get(&DataKey::Escrow)
        .ok_or(Error::NotInitialized)?;
    ttl::extend_instance(env);
    Ok(escrow)
}

/// Loads the agreement without changing ledger state.
pub fn load_view(env: &Env) -> Result<Escrow, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Escrow)
        .ok_or(Error::NotInitialized)
}

/// Persists the agreement and extends the instance TTL.
pub fn save(env: &Env, escrow: &Escrow) {
    env.storage().instance().set(&DataKey::Escrow, escrow);
    ttl::extend_instance(env);
}
