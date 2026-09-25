use soroban_sdk::{contracttype, Env};
use sororail_common::{storage as ttl, Error};

use crate::types::Authorization;

#[contracttype]
pub enum DataKey {
    /// The single authorization held by this instance.
    Auth,
}

/// Whether `authorize` has run.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Auth)
}

/// Loads the authorization for a mutating operation and extends the instance TTL.
pub fn load(env: &Env) -> Result<Authorization, Error> {
    let auth = env
        .storage()
        .instance()
        .get(&DataKey::Auth)
        .ok_or(Error::NotInitialized)?;
    ttl::extend_instance(env);
    Ok(auth)
}

/// Loads the authorization without changing ledger state.
pub fn load_view(env: &Env) -> Result<Authorization, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Auth)
        .ok_or(Error::NotInitialized)
}

/// Persists the authorization and extends the instance TTL.
pub fn save(env: &Env, auth: &Authorization) {
    env.storage().instance().set(&DataKey::Auth, auth);
    ttl::extend_instance(env);
}
