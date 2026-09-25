use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error};

use crate::types::Grant;

#[contracttype]
pub enum DataKey {
    /// The single grant held by this instance.
    Grant,
}

/// Whether `create` has run.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Grant)
}

/// Loads the grant for a mutating operation and extends the instance TTL.
pub fn load(env: &Env) -> Result<Grant, Error> {
    let grant = env
        .storage()
        .instance()
        .get(&DataKey::Grant)
        .ok_or(Error::NotInitialized)?;
    ttl::extend_instance(env);
    Ok(grant)
}

/// Loads the grant without changing ledger state.
pub fn load_view(env: &Env) -> Result<Grant, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Grant)
        .ok_or(Error::NotInitialized)
}

/// Persists the grant and extends the instance TTL.
pub fn save(env: &Env, grant: &Grant) {
    env.storage().instance().set(&DataKey::Grant, grant);
    ttl::extend_instance(env);
}
